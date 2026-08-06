//! Minimal source-only fixture for Fluid's bridge orientation acceptance test.

use openinfer::{FinishReason, GenerateRequest, Scheduler, TokenEvent};
use vllm::{EngineCoreOutputs, EngineCoreRequest, EngineCoreRequestType, FrontendSender};

fn handle_message(
    kind: EngineCoreRequestType,
    request: EngineCoreRequest,
    scheduler: &Scheduler,
    frontend_tx: &FrontendSender,
) {
    match kind {
        EngineCoreRequestType::Add => start_request(request, scheduler, frontend_tx),
        EngineCoreRequestType::Utility => send_utility_response(frontend_tx),
        EngineCoreRequestType::Abort => cleanup_streams(),
    }
}

fn start_request(
    request: EngineCoreRequest,
    scheduler: &Scheduler,
    frontend_tx: &FrontendSender,
) {
    if request.prompt_token_ids.is_empty() {
        send_terminal_output(frontend_tx, request.request_id, FinishReason::Error);
        return;
    }
    let trace_parent = start_trace(&request.request_id);
    let lora_adapter = load_lora(&request);
    scheduler.submit(GenerateRequest {
        request_id: request.request_id,
        prompt_tokens: request.prompt_token_ids,
        trace_parent,
        lora_adapter,
    });
}

fn dispatch_burst(event: TokenEvent, frontend_tx: &FrontendSender) {
    let outputs = reduce_request(event);
    frontend_tx.send(outputs);
}

fn reduce_request(event: TokenEvent) -> EngineCoreOutputs {
    match event {
        TokenEvent::Token(id) => EngineCoreOutputs::token(id),
        TokenEvent::Finished(reason) => EngineCoreOutputs::finished(reason),
    }
}

fn send_terminal_output(
    frontend_tx: &FrontendSender,
    request_id: String,
    reason: FinishReason,
) {
    frontend_tx.send(EngineCoreOutputs::terminal(request_id, reason));
}

async fn wait_for_ipc_endpoint(address: &str) {
    while !ipc_path(address).exists() {
        tokio::task::yield_now().await;
    }
}

fn publish_scheduler_stats(snapshot: SchedulerSnapshot, frontend_tx: &FrontendSender) {
    frontend_tx.send(EngineCoreOutputs::statistics(snapshot));
}

fn start_trace(request_id: &str) -> TraceContext {
    TraceContext::root(request_id)
}

fn load_lora(request: &EngineCoreRequest) -> Option<LoraAdapter> {
    request.sampling_params.lora_adapter()
}

fn send_utility_response(frontend_tx: &FrontendSender) {
    frontend_tx.send(EngineCoreOutputs::utility(false));
}

fn cleanup_streams() {
    STREAMS.with(|streams| streams.borrow_mut().clear());
}
