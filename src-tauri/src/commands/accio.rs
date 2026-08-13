use crate::accio_config;
use crate::store::AppState;
use serde::Serialize;
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

fn accio_app_path() -> &'static str {
    "/Applications/Accio.app"
}

fn accio_is_running() -> bool {
    Command::new("pgrep")
        .args(["-f", "/Applications/Accio.app/Contents/MacOS/Accio"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn kill_accio() -> Result<(), String> {
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

#[tauri::command]
pub async fn launch_accio(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] forceRestart: Option<bool>,
) -> Result<AccioLaunchResult, String> {
    if !std::path::Path::new(accio_app_path()).exists() {
        return Err("未找到 /Applications/Accio.app".to_string());
    }
    if !cfg!(target_os = "macos") {
        return Err("当前版本的 Accio 启动器仅支持 macOS".to_string());
    }
    let running = accio_is_running();
    if running && !forceRestart.unwrap_or(false) {
        return Err("accio_already_running".to_string());
    }
    if running {
        kill_accio()?;
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
            .ok_or_else(|| "未配置 Accio 供应商".to_string())?
    } else {
        providers
            .values()
            .next()
            .cloned()
            .ok_or_else(|| "未配置 Accio 供应商".to_string())?
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

    let output = Command::new("/usr/bin/open")
        .arg("-n")
        .arg("-a")
        .arg(accio_app_path())
        .arg("--env")
        .arg(format!("GATEWAY_BASE_URL={gateway_base_url}"))
        .arg("--env")
        .arg(format!("ADK_BASE_URL={adk_base_url}"))
        .arg("--env")
        .arg(format!("ADK_MODEL={model}"))
        .arg("--env")
        .arg(format!("ADK_TENANT={tenant}"))
        .arg("--env")
        .arg("ACCIO_GATEWAY_URL=".to_string() + &gateway_base_url)
        .arg("--env")
        .arg("NO_PROXY=127.0.0.1,localhost")
        .output()
        .map_err(|error| format!("启动 Accio 失败: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "启动 Accio 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(AccioLaunchResult {
        provider_id: provider_id.clone(),
        model,
        gateway_base_url,
        adk_base_url,
        launched: true,
    })
}
