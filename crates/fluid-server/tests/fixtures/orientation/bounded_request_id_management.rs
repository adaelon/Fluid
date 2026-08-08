type RequestId = u64;

struct RequestState {
    next_request_id: RequestId,
}

fn dispatch_request(state: &mut RequestState, payload: &str) -> RequestId {
    let request_id = state.next_request_id;
    state.next_request_id += 1;
    send_request(request_id, payload);
    request_id
}

fn archive_request_payload(payload: &str) -> usize {
    // __FLUID_OVERSIZED_ARCHIVE_BODY__
    payload.len()
}
