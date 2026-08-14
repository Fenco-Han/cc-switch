use crate::accio_config;
use crate::store::AppState;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccioLaunchResult {
    provider_id: String,
    model: String,
    gateway_base_url: String,
    adk_base_url: String,
    launched: bool,
}

#[tauri::command]
pub fn get_accio_live_provider_ids() -> Result<Vec<String>, String> {
    accio_config::get_providers()
        .map(|providers| providers.keys().cloned().collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_accio_live_provider(
    #[allow(non_snake_case)] providerId: String,
) -> Result<Option<serde_json::Value>, String> {
    accio_config::get_provider(&providerId).map_err(|e| e.to_string())
}

// ============================================================================
// 平台相关的 Accio 应用定位 / 运行检测 / 终止 / 启动
// ============================================================================

/// 解析 Accio 桌面应用入口路径。
///
/// 优先级：
///   1. `ACCIO_APP_PATH` 环境变量（若指向存在的文件）
///   2. 设置里持久化的 `accio_app_path`
///   3. 平台默认路径 / 常见安装位置自动发现
///
/// 返回 `None` 表示未能定位到已安装的 Accio。
fn resolve_accio_app_path() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("ACCIO_APP_PATH") {
        let value = raw.to_string_lossy().trim().to_string();
        let candidate = PathBuf::from(&value);
        let exists = if cfg!(target_os = "macos") {
            candidate.exists()
        } else {
            candidate.is_file()
        };
        if !value.is_empty() && exists {
            return Some(candidate);
        }
    }
    if let Some(configured) = crate::settings::get_accio_app_override_path() {
        let exists = if cfg!(target_os = "macos") {
            configured.exists()
        } else {
            configured.is_file()
        };
        if exists {
            return Some(configured);
        }
    }
    platform_default_accio_path()
}

/// 平台默认安装路径。
#[cfg(target_os = "macos")]
fn platform_default_accio_path() -> Option<PathBuf> {
    Some(PathBuf::from("/Applications/Accio.app"))
}

#[cfg(target_os = "windows")]
fn platform_default_accio_path() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local);
        candidates.push(base.join("Programs").join("Accio").join("Accio.exe"));
        candidates.push(base.join("Accio").join("Accio.exe"));
    }
    if let Some(prog) = std::env::var_os("PROGRAMFILES") {
        candidates.push(PathBuf::from(prog).join("Accio").join("Accio.exe"));
    }
    if let Some(prog86) = std::env::var_os("PROGRAMFILES(X86)") {
        candidates.push(PathBuf::from(prog86).join("Accio").join("Accio.exe"));
    }
    // 部分用户把应用装在自定义盘符的 `Program Files`（如 D:\Program Files\Accio）。
    // 兜底：遍历常见卷根下的 `Program Files` / `Program Files (x86)`。
    candidates.append(&mut well_known_drive_program_files());

    candidates.into_iter().find(|p| p.is_file())
}

/// 扫描各卷的 `Program Files` 目录下是否装有 Accio。
#[cfg(target_os = "windows")]
fn well_known_drive_program_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for base in ["C:\\", "D:\\", "E:\\"] {
        let root = PathBuf::from(base);
        if !root.exists() {
            continue;
        }
        for folder in ["Program Files", "Program Files (x86)"] {
            let dir = root.join(folder);
            if dir.is_dir() {
                out.push(dir.join("Accio").join("Accio.exe"));
            }
        }
    }
    out
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_default_accio_path() -> Option<PathBuf> {
    None
}

/// 进程镜像名（Windows 用）——取可执行文件文件名。
#[cfg(target_os = "windows")]
fn accio_image_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Accio.exe".to_string())
}

#[cfg(target_os = "macos")]
fn accio_is_running(_path: &Path) -> bool {
    Command::new("pgrep")
        .args(["-f", "/Applications/Accio.app/Contents/MacOS/Accio"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn accio_is_running(path: &Path) -> bool {
    let image = accio_image_name(path);
    let filter = format!("IMAGENAME eq {image}");
    let output = Command::new("tasklist")
        .args(["/FI", filter.as_str(), "/NH"])
        .output();
    match output {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines()
                .any(|line| line.trim_start().to_ascii_lowercase().starts_with(&image.to_ascii_lowercase()))
        }
        Err(_) => false,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn accio_is_running(_path: &Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn kill_accio(_path: &Path) -> Result<(), String> {
    let output = Command::new("pkill")
        .args(["-f", "/Applications/Accio.app/Contents/MacOS/Accio"])
        .output()
        .map_err(|error| format!("终止 Accio 失败: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "终止 Accio 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn kill_accio(path: &Path) -> Result<(), String> {
    let image = accio_image_name(path);
    let output = Command::new("taskkill")
        .args(["/F", "/IM", image.as_str()])
        .output()
        .map_err(|error| format!("终止 Accio 失败: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "终止 Accio 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn kill_accio(_path: &Path) -> Result<(), String> {
    Err("当前平台的 Accio 启动器未实现".to_string())
}

/// 以指定环境变量启动 Accio。
#[cfg(target_os = "macos")]
fn launch_accio_process(path: &Path, envs: &[(&str, String)]) -> Result<(), String> {
    let mut command = Command::new("/usr/bin/open");
    command.arg("-n").arg("-a").arg(path);
    for (key, value) in envs {
        command.arg("--env").arg(format!("{key}={value}"));
    }
    let output = command
        .output()
        .map_err(|error| format!("启动 Accio 失败: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "启动 Accio 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// 直接以带环境变量的方式启动 Accio（Windows）。
///
/// 不用 `cmd /C set VAR=... & start "" "exe"`——`cmd /C` 会对整条含引号的内容
/// 做重新引号解析，路径里带空格/反斜杠时会把孤立 `\` 或路径片段误当成要执行的
/// 程序，报「Windows 找不到 '\\' 文件」。改用 `std::process::Command` 直接 spawn
/// 并通过 `.env()` 注入环境变量，无控制台闪烁、无引号转义问题。Accio 是 GUI
/// 子系统程序，spawn 后不会挂起控制台窗口，且与父进程相互独立。
#[cfg(target_os = "windows")]
fn launch_accio_process(path: &Path, envs: &[(&str, String)]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut command = Command::new(path);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.creation_flags(CREATE_NO_WINDOW);

    match command.spawn() {
        Ok(child) => {
            log::info!("[accio] spawned {:?} pid={}", path, child.id());
            Ok(())
        }
        Err(error) => {
            log::error!("[accio] spawn {:?} failed: {error}", path);
            Err(format!("启动 Accio 失败: {error}"))
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn launch_accio_process(_path: &Path, _envs: &[(&str, String)]) -> Result<(), String> {
    Err("当前平台的 Accio 启动器未实现".to_string())
}

#[tauri::command]
pub async fn launch_accio(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] forceRestart: Option<bool>,
) -> Result<AccioLaunchResult, String> {
    if !cfg!(any(target_os = "macos", target_os = "windows")) {
        return Err("当前版本的 Accio 启动器仅支持 macOS 与 Windows".to_string());
    }

    let app_path = match resolve_accio_app_path() {
        Some(p) => {
            log::info!("[accio] resolved app path: {}", p.display());
            p
        }
        None => {
            log::error!("[accio] Accio app path not found on this machine");
            return Err(
                "未找到 Accio 应用，请确认已安装或在设置中配置 Accio 应用路径。".to_string(),
            );
        }
    };

    let running = accio_is_running(&app_path);
    log::info!("[accio] running={running} forceRestart={forceRestart:?} path={}", app_path.display());
    if running && !forceRestart.unwrap_or(false) {
        return Err("accio_already_running".to_string());
    }
    if running {
        kill_accio(&app_path)?;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }

    // 以「已添加」的供应商为准：从 live 配置读取全部供应商，
    // 取第一个作为 Accio 启动时的默认模型；完整模型目录由
    // `/accio/api/llm/config/v2` 一次性注入所有供应商的模型。
    let providers = accio_config::get_providers().map_err(|error| error.to_string())?;
    let provider = if providers.is_empty() {
        state
            .db
            .get_all_providers("accio")
            .map_err(|error| error.to_string())?
            .into_values()
            .next()
            .map(|provider| provider.settings_config)
            .ok_or_else(|| {
                log::error!("[accio] no accio provider configured");
                "未配置 Accio 供应商".to_string()
            })?
    } else {
        providers
            .values()
            .next()
            .cloned()
            .ok_or_else(|| {
                log::error!("[accio] live providers map is non-empty but has no values");
                "未配置 Accio 供应商".to_string()
            })?
    };

    let provider_id = provider
        .get("name")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_default();
    let model = provider
        .get("model")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            provider
                .get("models")
                .and_then(|value| value.as_array())
                .and_then(|models| models.first())
                .and_then(|value| value.get("id").or_else(|| value.get("model")))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| provider_id.clone());

    let proxy = state
        .proxy_service
        .start()
        .await
        .map_err(|error| format!("启动 CC Switch 本地代理失败: {error}"))?;
    let gateway_base_url = format!("http://{}:{}/accio", proxy.address, proxy.port);
    let adk_base_url = format!("{gateway_base_url}/api/adk/llm");
    let tenant = provider
        .get("name")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| provider_id.clone());

    let envs: Vec<(&str, String)> = vec![
        ("GATEWAY_BASE_URL", gateway_base_url.clone()),
        ("ADK_BASE_URL", adk_base_url.clone()),
        ("ADK_MODEL", model.clone()),
        ("ADK_TENANT", tenant),
        ("ACCIO_GATEWAY_URL", gateway_base_url.clone()),
        ("NO_PROXY", "127.0.0.1,localhost".to_string()),
    ];

    log::info!(
        "[accio] launching provider_id={provider_id} model={model} gateway={gateway_base_url}"
    );
    let launch_result = launch_accio_process(&app_path, &envs);
    if let Err(e) = &launch_result {
        log::error!("[accio] launch_accio_process failed: {e}");
    }
    launch_result?;

    Ok(AccioLaunchResult {
        provider_id: provider_id.clone(),
        model,
        gateway_base_url,
        adk_base_url,
        launched: true,
    })
}
