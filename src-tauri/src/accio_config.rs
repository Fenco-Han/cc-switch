//! Accio 配置文件读写模块
//!
//! Accio 是阿里电商 AI 桌面应用（Electron，`@phoenix/desktop`），模型请求走
//! `${gateway}/api/adk/llm`（阿里 ADK 协议），没有 Codex `config.toml` /
//! Hermes `config.yaml` 那种可自由写入的明文供应商列表。
//!
//! CC Switch 采用「自有供应商配置文件 + 环境变量注入」的方案：
//!   - 供应商列表由 CC Switch 维护在 `~/.accio/cc-switch-providers.json`
//!     （JSON，`{ "providers": [ ... ] }`），字段与其它 app 对齐
//!     （`name` / `base_url` / `api_key` / `api_format` / `model`）。
//!   - 切换供应商时写入该文件并备份。
//!
//! 当前阶段先打通供应商 CRUD 与切换这条主链路；实际启动 Accio 时的
//! 环境变量注入层（`GATEWAY_BASE_URL` / `ADK_MODEL` 等）由后续启动器
//! 工作接管，尚未在本模块中落盘。
//!
//! ## 配置文件结构示例
//!
//! ```json
//! {
//!   "providers": [
//!     {
//!       "id": "cc-...",
//!       "name": "deepseek",
//!       "base_url": "https://api.deepseek.com",
//!       "api_key": "sk-...",
//!       "api_format": "openai_chat",
//!       "model": "deepseek-chat"
//!     }
//!   ]
//! }
//! ```

use crate::config::{atomic_write, get_app_config_dir, get_home_dir};
use crate::error::AppError;
use crate::settings::effective_backup_retain_count;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// ============================================================================
// Path Functions
// ============================================================================

/// 获取 Accio 配置目录。
///
/// 解析顺序：
///   1. `ACCIO_HOME` 环境变量（trim 后非空）
///   2. CC Switch 的设备级目录覆盖
///   3. 平台默认 `~/.accio`
pub fn get_accio_dir() -> PathBuf {
    if let Some(raw) = std::env::var_os("ACCIO_HOME") {
        let value = raw.to_string_lossy();
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Some(configured) = crate::settings::get_accio_override_dir() {
        return configured;
    }
    get_home_dir().join(".accio")
}

/// CC Switch 管理的 Accio 供应商配置文件路径。
pub fn get_accio_providers_path() -> PathBuf {
    get_accio_dir().join("cc-switch-providers.json")
}

fn accio_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

// ============================================================================
// Type Definitions
// ============================================================================

/// Accio 写入结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccioWriteOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
}

// ============================================================================
// Core Read Functions
// ============================================================================

/// 读取 Accio 供应商配置文件为 serde_json::Value。
///
/// 如果文件不存在或为空，返回空的 `{ "providers": [] }`。
pub fn read_accio_providers() -> Result<serde_json::Value, AppError> {
    let path = get_accio_providers_path();
    if !path.exists() {
        return Ok(serde_json::json!({ "providers": [] }));
    }

    let content = fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
    if content.trim().is_empty() {
        return Ok(serde_json::json!({ "providers": [] }));
    }

    serde_json::from_str(&content)
        .map_err(|e| AppError::Config(format!("Accio 供应商配置文件 JSON 解析失败: {e}")))
}

// ============================================================================
// Backup Functions
// ============================================================================

fn create_accio_backup(source: &str) -> Result<PathBuf, AppError> {
    let backup_dir = get_app_config_dir().join("backups").join("accio");
    fs::create_dir_all(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))?;

    let base_id = format!("accio_{}", Local::now().format("%Y%m%d_%H%M%S"));
    let mut filename = format!("{base_id}.json");
    let mut backup_path = backup_dir.join(&filename);
    let mut counter = 1;

    while backup_path.exists() {
        filename = format!("{base_id}_{counter}.json");
        backup_path = backup_dir.join(&filename);
        counter += 1;
    }

    atomic_write(&backup_path, source.as_bytes())?;
    cleanup_accio_backups(&backup_dir)?;
    Ok(backup_path)
}

fn cleanup_accio_backups(dir: &Path) -> Result<(), AppError> {
    let retain = effective_backup_retain_count();
    let mut entries = fs::read_dir(dir)
        .map_err(|e| AppError::io(dir, e))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    if entries.len() <= retain {
        return Ok(());
    }

    entries.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());
    let remove_count = entries.len().saturating_sub(retain);
    for entry in entries.into_iter().take(remove_count) {
        if let Err(err) = fs::remove_file(entry.path()) {
            log::warn!(
                "Failed to remove old Accio config backup {}: {err}",
                entry.path().display()
            );
        }
    }

    Ok(())
}

// ============================================================================
// Provider Functions
// ============================================================================

/// 获取所有 Accio 供应商，以 name 为键。
pub fn get_providers() -> Result<serde_json::Map<String, serde_json::Value>, AppError> {
    let config = read_accio_providers()?;
    let mut map = serde_json::Map::new();

    if let Some(seq) = config.get("providers").and_then(|v| v.as_array()) {
        for item in seq {
            if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                map.insert(name.to_string(), item.clone());
            }
        }
    }

    Ok(map)
}

/// 获取单个供应商。
pub fn get_provider(name: &str) -> Result<Option<serde_json::Value>, AppError> {
    Ok(get_providers()?.get(name).cloned())
}

/// 设置（upsert）一个供应商，按 `name` 匹配。
pub fn set_provider(
    name: &str,
    provider_config: serde_json::Value,
) -> Result<AccioWriteOutcome, AppError> {
    let _guard = accio_write_lock().lock()?;

    let config = read_accio_providers()?;
    let mut providers: Vec<serde_json::Value> = config
        .get("providers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut normalized = provider_config;
    if let Some(obj) = normalized.as_object_mut() {
        obj.insert("name".to_string(), serde_json::json!(name));
    }

    if let Some(existing) = providers
        .iter_mut()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
    {
        if let (Some(existing_map), Some(new_map)) = (existing.as_object(), normalized.as_object())
        {
            let mut merged = existing_map.clone();
            for (k, v) in new_map {
                merged.insert(k.clone(), v.clone());
            }
            *existing = serde_json::Value::Object(merged);
        } else {
            *existing = normalized;
        }
    } else {
        providers.push(normalized);
    }

    write_providers(&providers)
}

/// 删除一个供应商，按 `name` 匹配。
pub fn remove_provider(name: &str) -> Result<AccioWriteOutcome, AppError> {
    let _guard = accio_write_lock().lock()?;

    let config = read_accio_providers()?;
    let mut providers: Vec<serde_json::Value> = config
        .get("providers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let original_len = providers.len();
    providers.retain(|p| p.get("name").and_then(|n| n.as_str()) != Some(name));
    if providers.len() == original_len {
        return Ok(AccioWriteOutcome::default());
    }

    write_providers(&providers)
}

/// 切换供应商时应用默认配置。Accio 是累加式（additive），与 set_provider 等价。
pub fn apply_switch_defaults(
    name: &str,
    provider_config: serde_json::Value,
) -> Result<AccioWriteOutcome, AppError> {
    set_provider(name, provider_config)
}

/// 将 providers 数组写回配置文件（带备份）。调用方须持有写锁。
fn write_providers(providers: &[serde_json::Value]) -> Result<AccioWriteOutcome, AppError> {
    let path = get_accio_providers_path();
    let config = serde_json::json!({ "providers": providers });
    let new_raw = serde_json::to_string_pretty(&config)
        .map_err(|e| AppError::Config(format!("Accio 供应商配置序列化失败: {e}")))?;

    let old_raw = if path.exists() {
        fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };

    if old_raw == new_raw {
        return Ok(AccioWriteOutcome::default());
    }

    let backup_path = if !old_raw.trim().is_empty() {
        Some(create_accio_backup(&old_raw)?)
    } else {
        None
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    atomic_write(&path, new_raw.as_bytes())?;

    log::debug!("Accio providers written to {:?}", path);
    Ok(AccioWriteOutcome {
        backup_path: backup_path.map(|p| p.display().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    struct AccioHome {
        _dir: TempDir,
        original: Option<std::ffi::OsString>,
    }

    impl AccioHome {
        fn new() -> Self {
            let _dir = tempfile::tempdir().expect("create accio temp home");
            let original = std::env::var_os("ACCIO_HOME");
            std::env::set_var("ACCIO_HOME", _dir.path());
            Self { _dir, original }
        }
    }

    impl Drop for AccioHome {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var("ACCIO_HOME", value),
                None => std::env::remove_var("ACCIO_HOME"),
            }
        }
    }

    #[test]
    #[serial]
    fn accio_home_env_overrides_default_dir() {
        let guard = AccioHome::new();
        assert_eq!(get_accio_dir(), guard._dir.path());
    }

    #[test]
    #[serial]
    fn set_get_and_remove_provider_round_trip() {
        let _guard = AccioHome::new();
        set_provider(
            "deepseek",
            serde_json::json!({
                "name": "deepseek",
                "base_url": "https://api.deepseek.com",
                "api_key": "sk-test",
                "api_format": "openai_chat",
                "model": "deepseek-chat"
            }),
        )
        .expect("set provider");

        let provider = get_provider("deepseek")
            .expect("read providers")
            .expect("provider exists");
        assert_eq!(provider["base_url"], "https://api.deepseek.com");
        assert_eq!(provider["model"], "deepseek-chat");

        remove_provider("deepseek").expect("remove provider");
        assert!(get_provider("deepseek")
            .expect("read providers after removal")
            .is_none());
    }

    #[test]
    #[serial]
    fn set_provider_merges_without_dropping_unknown_fields() {
        let _guard = AccioHome::new();
        set_provider(
            "deepseek",
            serde_json::json!({
                "name": "deepseek",
                "base_url": "https://api.deepseek.com",
                "api_key": "sk-old",
                "custom_field": "keep-me"
            }),
        )
        .expect("initial set");

        set_provider(
            "deepseek",
            serde_json::json!({
                "name": "deepseek",
                "api_key": "sk-new"
            }),
        )
        .expect("update set");

        let provider = get_provider("deepseek").expect("read providers").unwrap();
        assert_eq!(provider["api_key"], "sk-new");
        assert_eq!(provider["base_url"], "https://api.deepseek.com");
        assert_eq!(provider["custom_field"], "keep-me");
    }

    #[test]
    #[serial]
    fn malformed_provider_file_is_an_error() {
        let _guard = AccioHome::new();
        std::fs::write(get_accio_providers_path(), "{ not json").expect("write malformed config");

        let error = get_providers().expect_err("malformed config should fail");
        assert!(error.to_string().contains("JSON"));
    }
}
