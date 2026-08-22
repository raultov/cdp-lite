use cdp_lite::client::CdpClient;
use cdp_lite::error::{CdpError, CdpResult};
use cdp_lite::event_filter::EventFilter;
use cdp_lite::protocol::{NoParams, WsResponse};
use log::debug;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
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

    let cdp_client = CdpClient::new("127.0.0.1:9222", Duration::from_secs(200)).await?;
    enable_debugger(&cdp_client).await?;

    let expression_evaluated_signal = Arc::new(Notify::new());
    spawn_debugger_task(cdp_client.clone(), expression_evaluated_signal.clone());

    cdp_client
        .send_raw_command("Page.navigate", json!({"url": "https://www.rust-lang.org"}))
        .await?;

    info!("Waiting for breakpoint resolution...");
    wait_for_breakpoint(expression_evaluated_signal).await;

    Ok(())
}

/// Prepares the connection to receive script and pause notifications.
async fn enable_debugger(client: &CdpClient) -> CdpResult<()> {
    client.send_raw_command("Page.enable", NoParams).await?;
    client.send_raw_command("Debugger.enable", NoParams).await?;

    Ok(())
}

/// Runs the debugger event loop in the background.
fn spawn_debugger_task(client: CdpClient, expression_evaluated: Arc<Notify>) -> JoinHandle<()> {
    let mut debug_events = client.on_domain("Debugger");

    tokio::spawn(async move {
        let result: CdpResult<()> =
            handle_debug_events(&client, &mut debug_events, &expression_evaluated).await;

        if let Err(e) = result {
            error!("Fatal error in Debugger task: {}", e);
        }
    })
}

async fn handle_debug_events(
    client: &CdpClient,
    debug_events: &mut EventFilter,
    expression_evaluated: &Notify,
) -> CdpResult<()> {
    let mut breakpoint_set = false;

    while let Some(Ok(event)) = debug_events.next().await {
        match event.method.as_deref() {
            Some("Debugger.scriptParsed") => {
                if !breakpoint_set && set_breakpoint(client, &event).await? {
                    breakpoint_set = true;
                    client.send_raw_command("Page.reload", NoParams).await?;
                }
            }
            Some("Debugger.paused") => {
                inspect_paused_frame(client, &event).await?;
                client.send_raw_command("Debugger.resume", NoParams).await?;
                expression_evaluated.notify_one();
            }
            Some(method) => debug!("Debugger method received: '{}'", method),
            None => {}
        }
    }

    Ok(())
}

/// Finds the script containing `nav_dropdown.value` and plants a breakpoint
/// on it, returning whether the breakpoint was set.
async fn set_breakpoint(client: &CdpClient, event: &WsResponse) -> CdpResult<bool> {
    let script_id = extract_from_value(&event.params, "scriptId")
        .ok_or_else(|| CdpError::InternalError("Script Id not found".to_string()))?;

    let Ok(script_result) = client
        .send_raw_command("Debugger.getScriptSource", json!({"scriptId": script_id}))
        .await
    else {
        return Ok(false);
    };

    let Some((line_number, column_number)) =
        extract_from_value(&script_result.result, "scriptSource")
            .and_then(|source| find_line_column(source, "nav_dropdown.value"))
    else {
        return Ok(false);
    };

    let hash = extract_from_value(&event.params, "hash")
        .ok_or_else(|| CdpError::InternalError("Hash Id not found".to_string()))?;

    client
        .send_raw_command(
            "Debugger.setBreakpointByUrl",
            json!({
                "lineNumber": line_number,
                "columnNumber": column_number,
                "scriptHash": hash
            }),
        )
        .await?;

    Ok(true)
}

/// Evaluates `current_lang` on the paused frame and logs the result.
async fn inspect_paused_frame(client: &CdpClient, event: &WsResponse) -> CdpResult<()> {
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
        .ok_or_else(|| CdpError::InternalError("Frame Id or Function Name not found".into()))?;

    let expression_result = client
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

    Ok(())
}

/// Awaits the debugger task's signal, warning instead of failing when the
/// page never hits the breakpoint.
async fn wait_for_breakpoint(signal: Arc<Notify>) {
    let result = timeout(Duration::from_secs(10), signal.notified()).await;
    match result {
        Ok(_) => info!("Breakpoint was resolved! Continuing main execution..."),
        Err(_) => warn!("Error: Timed out waiting for script."),
    }
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
