use std::collections::BTreeMap;

type RequestId = u64;

#[derive(Default)]
struct RequestRegistry {
    next_request_id: RequestId,
    labels: BTreeMap<RequestId, String>,
}

fn begin_request(registry: &mut RequestRegistry, label: String) -> RequestId {
    let request_id = allocate_request_id(registry);
    registry.labels.insert(request_id, label);
    request_id
}

fn allocate_request_id(registry: &mut RequestRegistry) -> RequestId {
    let request_id = registry.next_request_id;
    registry.next_request_id += 1;
    request_id
}

fn release_request_id(registry: &mut RequestRegistry, request_id: RequestId) -> Option<String> {
    registry.labels.remove(&request_id)
}

fn active_request_count(registry: &RequestRegistry) -> usize {
    registry.labels.len()
}
