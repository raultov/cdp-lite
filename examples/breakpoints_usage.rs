use cdp_lite::error::{CdpError, CdpResult};
use cdp_lite::protocol::NoParams;
use log::debug;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> CdpResult<()> {
    fmt()
        .pretty()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cdp_client =
        cdp_lite::client::CdpClient::new("127.0.0.1:9222", Duration::from_secs(200)).await?;
    let cdp_client_clone = cdp_client.clone();
    cdp_client.send_raw_command("Page.enable", NoParams).await?;
    cdp_client
        .send_raw_command("Debugger.enable", NoParams)
        .await?;

    let expression_evaluated_signal = Arc::new(Notify::new());
    let expression_evaluated_signal_clone = expression_evaluated_signal.clone();

    let mut debug_events = cdp_client.on_domain("Debugger");
    spawn(async move {
        let result: CdpResult<()> = async {
            let mut target_found = false;
            while let Some(Ok(event)) = debug_events.next().await {
                let method = event.method.as_deref();
                if let Some(method) = method {
                    match method {
                        "Debugger.scriptParsed" => {
                            if target_found {
                                continue;
                            }

                            let script_id = extract_from_value(&event.params, "scriptId")
                                .ok_or_else(|| {
                                    CdpError::InternalError("Script Id not found".to_string())
                                })?;
                            if let Ok(script_result) = cdp_client_clone
                                .send_raw_command(
                                    "Debugger.getScriptSource",
                                    json!({"scriptId": script_id}),
                                )
                                .await
                            {
                                if let Some((line_number, column_number)) =
                                    extract_from_value(&script_result.result, "scriptSource")
                                        .and_then(|s| find_line_column(s, "nav_dropdown.value"))
                                {
                                    let hash = extract_from_value(&event.params, "hash")
                                        .ok_or_else(|| {
                                            CdpError::InternalError("Hash Id not found".to_string())
                                        })?;

                                    cdp_client_clone
                                        .send_raw_command(
                                            "Debugger.setBreakpointByUrl",
                                            json!({
                                                "lineNumber": line_number,
                                                "columnNumber": column_number,
                                                "scriptHash": hash
                                            }),
                                        )
                                        .await?;

                                    target_found = true;
                                    cdp_client_clone
                                        .send_raw_command("Page.reload", NoParams)
                                        .await?;
                                }
                            }
                        }
                        "Debugger.paused" => {
                            let (call_frame_id, function_name) = event
                                .params
                                .as_ref()
                                .and_then(|p| p.get("callFrames"))
                                .and_then(|frames| frames.as_array())
                                .and_then(|frames| frames.first())
                                .and_then(|first_frame| {
                                    first_frame
                                        .get("callFrameId")
                                        .and_then(|id| id.as_str())
                                        .zip(
                                            first_frame
                                                .get("functionName")
                                                .and_then(|name| name.as_str()),
                                        )
                                })
                                .ok_or_else(|| {
                                    CdpError::InternalError(
                                        "Frame Id or Function Name not found".into(),
                                    )
                                })?;

                            let expression_result = cdp_client_clone
                                .send_raw_command(
                                    "Debugger.evaluateOnCallFrame",
                                    json!({
                                        "callFrameId": call_frame_id,
                                        "returnByValue": true,
                                        "expression": "current_lang"
                                    }),
                                )
                                .await?;

                            info!(
                                "Expression result: {:?} from Function Name: {}",
                                expression_result, function_name
                            );

                            cdp_client_clone
                                .send_raw_command("Debugger.resume", NoParams)
                                .await?;
                            expression_evaluated_signal_clone.notify_one();
                        }
                        _ => {
                            debug!("Debugger method received: '{}'", method);
                        }
                    }
                }
            }

            Ok(())
        }
        .await;

        if let Err(e) = result {
            error!("Fatal error in Debugger task: {}", e);
        }
    });

    cdp_client
        .send_raw_command("Page.navigate", json!({"url": "https://www.rust-lang.org"}))
        .await?;

    info!("Waiting for breakpoint resolution...");
    let result = timeout(
        Duration::from_secs(10),
        expression_evaluated_signal.notified(),
    )
    .await;
    match result {
        Ok(_) => info!("Breakpoint was resolved! Continuing main execution..."),
        Err(_) => warn!("Error: Timed out waiting for script."),
    }

    Ok(())
}

fn extract_from_value<'a>(value: &'a Option<Value>, param_name: &str) -> Option<&'a str> {
    value
        .as_ref()
        .and_then(|p| p.get(param_name))
        .and_then(|v| v.as_str())
}

fn find_line_column(source: &str, pattern: &str) -> Option<(usize, usize)> {
    debug!("Searching for pattern {} in Source: {}", pattern, source);
    let byte_index = source.find(pattern)?;
    let prefix = &source[..byte_index];
    let line_number = prefix.lines().count().saturating_sub(1);
    let column_number = prefix.lines().last().map(|line| line.len()).unwrap_or(0);

    Some((line_number, column_number))
}
