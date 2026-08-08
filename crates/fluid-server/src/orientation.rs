//! File-orientation protocol types, deterministic validation, and stable
//! identities (S-ORI-1).
//!
//! LLM and route integration deliberately stay outside this module, so every
//! full-source or bounded-source producer/consumer shares one model-independent
//! boundary. Source remains the truth: graph data can affect the cache
//! identity, while every accepted evidence anchor must point back into the active
//! file and its current line range.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

pub const ORIENTATION_SCHEMA_VERSION: u32 = 1;
pub const ORIENTATION_PROMPT_VERSION: &str = "orientation-p3";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeEvidenceRef {
    pub id: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActorBoundary {
    InsideFile,
    Project,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationActor {
    pub id: String,
    pub name: String,
    pub role: String,
    pub boundary: ActorBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationType {
    pub name: String,
    pub owner_actor_id: String,
    pub meaning: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationFlowStep {
    pub from_actor_id: String,
    pub via: String,
    pub payload: String,
    pub to_actor_id: String,
    pub why: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrientationFlowKind {
    Request,
    Response,
    Control,
    Stats,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationFlow {
    pub id: String,
    pub name: String,
    pub kind: OrientationFlowKind,
    pub why: String,
    pub steps: Vec<OrientationFlowStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionLane {
    Core,
    Supporting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionRole {
    pub fn_id: String,
    pub lane: FunctionLane,
    #[serde(default)]
    pub flow_ids: Vec<String>,
    pub stage: String,
    pub receives_from_actor_ids: Vec<String>,
    pub consumes: Vec<String>,
    pub sends_to_actor_ids: Vec<String>,
    pub produces: Vec<String>,
    pub why: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportingCapability {
    pub name: String,
    pub why: String,
    pub function_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkthroughStep {
    pub text: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationWalkthrough {
    pub title: String,
    pub input: String,
    pub steps: Vec<WalkthroughStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationInvariant {
    pub text: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrientationCoverageMode {
    FullSource,
    BoundedSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationCoverage {
    pub mode: OrientationCoverageMode,
    pub omitted_function_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOrientationCard {
    pub schema_version: u32,
    pub orientation_id: String,
    pub file_path: String,
    pub purpose: String,
    pub actors: Vec<OrientationActor>,
    pub types: Vec<OrientationType>,
    pub core_flows: Vec<OrientationFlow>,
    pub supporting_capabilities: Vec<SupportingCapability>,
    pub function_roles: Vec<FunctionRole>,
    pub walkthrough: OrientationWalkthrough,
    pub invariants: Vec<OrientationInvariant>,
    pub evidence: Vec<CodeEvidenceRef>,
    pub coverage: OrientationCoverage,
}

/// Model-produced stage-A fields. Backend-owned identity and coverage facts are
/// deliberately absent so they can only enter through the final merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationSkeleton {
    pub purpose: String,
    pub actors: Vec<OrientationActor>,
    pub types: Vec<OrientationType>,
    pub core_flows: Vec<OrientationFlow>,
    pub walkthrough: OrientationWalkthrough,
    pub invariants: Vec<OrientationInvariant>,
    pub evidence: Vec<CodeEvidenceRef>,
}

/// Model-produced stage-B fields for exactly one backend-defined function
/// batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrientationRoleBatch {
    pub function_roles: Vec<FunctionRole>,
    pub supporting_capabilities: Vec<SupportingCapability>,
}

/// Source projection selected by the backend for one function in a role
/// batch. Signature-only views intentionally carry no function body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrientationFunctionSourceView {
    Exact {
        fn_id: String,
        numbered_source: String,
    },
    SignatureOnly {
        fn_id: String,
        numbered_signature: String,
    },
}

impl OrientationFunctionSourceView {
    fn fn_id(&self) -> &str {
        match self {
            Self::Exact { fn_id, .. } | Self::SignatureOnly { fn_id, .. } => fn_id,
        }
    }

    fn numbered_source(&self) -> &str {
        match self {
            Self::Exact {
                numbered_source, ..
            } => numbered_source,
            Self::SignatureOnly {
                numbered_signature, ..
            } => numbered_signature,
        }
    }
}

/// Backend-frozen boundary for one stage-B request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrientationRoleBatchSpec {
    pub index: usize,
    pub fn_ids: Vec<String>,
    pub source_views: Vec<OrientationFunctionSourceView>,
}

/// Backend-owned fields injected exactly once after all role batches validate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrientationBackendFacts {
    pub schema_version: u32,
    pub orientation_id: String,
    pub file_path: String,
    pub coverage: OrientationCoverage,
    pub roster_fn_ids: Vec<String>,
}

/// Backend-owned facts used to reject invented paths, line numbers, and
/// functions. The roster must already be verified against the current source.
pub struct OrientationValidationContext<'a> {
    pub file_path: &'a str,
    pub source: &'a str,
    pub roster_fn_ids: &'a [String],
    /// Canonical fnId -> 1-based source span. Full-source cards do not need it;
    /// bounded-source cards use it to reject evidence from omitted bodies.
    pub roster_line_ranges: Option<&'a BTreeMap<String, [u32; 2]>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrientationValidationError {
    message: String,
}

impl OrientationValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for OrientationValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OrientationValidationError {}

impl FileOrientationCard {
    pub fn validate(
        &self,
        context: &OrientationValidationContext<'_>,
    ) -> Result<(), OrientationValidationError> {
        if self.schema_version != ORIENTATION_SCHEMA_VERSION {
            return invalid(format!(
                "unsupported schemaVersion {}; expected {ORIENTATION_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        nonblank("orientationId", &self.orientation_id)?;
        nonblank("filePath", &self.file_path)?;
        descriptive("purpose", &self.purpose)?;

        let active_path = normalize_project_path(context.file_path)
            .ok_or_else(|| OrientationValidationError::new("active file path is unsafe"))?;
        let card_path = normalize_project_path(&self.file_path)
            .ok_or_else(|| OrientationValidationError::new("card filePath is unsafe"))?;
        if card_path != active_path {
            return invalid(format!(
                "card filePath {card_path:?} does not match active file {active_path:?}"
            ));
        }

        if self.actors.is_empty() {
            return invalid("actors must not be empty");
        }
        let actor_ids = unique_ids("actor", self.actors.iter().map(|actor| &actor.id))?;
        for actor in &self.actors {
            nonblank("actor.name", &actor.name)?;
            descriptive("actor.role", &actor.role)?;
        }

        if self.evidence.is_empty() {
            return invalid("evidence must not be empty");
        }
        let evidence_ids = unique_ids(
            "evidence",
            self.evidence.iter().map(|evidence| &evidence.id),
        )?;
        let source_lines = context.source.split('\n').count() as u32;
        for evidence in &self.evidence {
            let evidence_path = normalize_project_path(&evidence.file_path).ok_or_else(|| {
                OrientationValidationError::new(format!(
                    "evidence {} has an unsafe filePath",
                    evidence.id
                ))
            })?;
            if evidence_path != active_path {
                return invalid(format!(
                    "evidence {} points to {evidence_path:?}, not the active file",
                    evidence.id
                ));
            }
            if evidence.start_line == 0
                || evidence.start_line > evidence.end_line
                || evidence.end_line > source_lines
            {
                return invalid(format!(
                    "evidence {} has invalid line range {}..={} for {source_lines} lines",
                    evidence.id, evidence.start_line, evidence.end_line
                ));
            }
        }

        if self.core_flows.is_empty() {
            return invalid("coreFlows must not be empty");
        }
        let flow_ids = unique_ids("flow", self.core_flows.iter().map(|flow| &flow.id))?;
        for flow in &self.core_flows {
            nonblank("flow.name", &flow.name)?;
            descriptive("flow.why", &flow.why)?;
            if flow.steps.is_empty() {
                return invalid(format!("core flow {} has no steps", flow.id));
            }
            for (index, step) in flow.steps.iter().enumerate() {
                nonblank("flow.step.via", &step.via)?;
                nonblank("flow.step.payload", &step.payload)?;
                descriptive("flow.step.why", &step.why)?;
                require_ref(
                    "actor",
                    &step.from_actor_id,
                    &actor_ids,
                    &format!("flow {} step {index} fromActorId", flow.id),
                )?;
                require_ref(
                    "actor",
                    &step.to_actor_id,
                    &actor_ids,
                    &format!("flow {} step {index} toActorId", flow.id),
                )?;
                require_nonempty_refs(
                    "evidence",
                    &step.evidence_ids,
                    &evidence_ids,
                    &format!("flow {} step {index}", flow.id),
                )?;
            }
        }

        for orientation_type in &self.types {
            nonblank("type.name", &orientation_type.name)?;
            descriptive("type.meaning", &orientation_type.meaning)?;
            require_ref(
                "actor",
                &orientation_type.owner_actor_id,
                &actor_ids,
                &format!("type {} ownerActorId", orientation_type.name),
            )?;
        }

        let roster_ids = unique_ids("roster function", context.roster_fn_ids.iter())?;
        let mut role_lanes = BTreeMap::new();
        for role in &self.function_roles {
            nonblank("functionRole.fnId", &role.fn_id)?;
            if !roster_ids.contains(role.fn_id.as_str()) {
                return invalid(format!(
                    "functionRole references unknown fnId {:?}",
                    role.fn_id
                ));
            }
            if role_lanes.insert(role.fn_id.as_str(), role.lane).is_some() {
                return invalid(format!(
                    "function {:?} has overlapping core/supporting roles",
                    role.fn_id
                ));
            }
            descriptive("functionRole.stage", &role.stage)?;
            descriptive("functionRole.why", &role.why)?;
            if role.lane == FunctionLane::Core && role.flow_ids.is_empty() {
                return invalid(format!(
                    "core function {:?} must reference at least one flow",
                    role.fn_id
                ));
            }
            require_refs(
                "flow",
                &role.flow_ids,
                &flow_ids,
                &format!("functionRole {}", role.fn_id),
            )?;
            require_refs(
                "actor",
                &role.receives_from_actor_ids,
                &actor_ids,
                &format!("functionRole {} receivesFromActorIds", role.fn_id),
            )?;
            require_refs(
                "actor",
                &role.sends_to_actor_ids,
                &actor_ids,
                &format!("functionRole {} sendsToActorIds", role.fn_id),
            )?;
            require_refs(
                "evidence",
                &role.evidence_ids,
                &evidence_ids,
                &format!("functionRole {}", role.fn_id),
            )?;
        }
        for fn_id in &roster_ids {
            if !role_lanes.contains_key(fn_id) {
                return invalid(format!("roster function {fn_id:?} has no functionRole"));
            }
        }

        for capability in &self.supporting_capabilities {
            nonblank("supportingCapability.name", &capability.name)?;
            descriptive("supportingCapability.why", &capability.why)?;
            if capability.function_ids.is_empty() {
                return invalid(format!(
                    "supporting capability {:?} has no functions",
                    capability.name
                ));
            }
            for fn_id in &capability.function_ids {
                let Some(lane) = role_lanes.get(fn_id.as_str()) else {
                    return invalid(format!(
                        "supporting capability {:?} references unknown fnId {fn_id:?}",
                        capability.name
                    ));
                };
                if *lane != FunctionLane::Supporting {
                    return invalid(format!(
                        "core function {fn_id:?} also appears in supporting capabilities"
                    ));
                }
            }
            require_nonempty_refs(
                "evidence",
                &capability.evidence_ids,
                &evidence_ids,
                &format!("supporting capability {}", capability.name),
            )?;
        }

        descriptive("walkthrough.title", &self.walkthrough.title)?;
        nonblank("walkthrough.input", &self.walkthrough.input)?;
        if self.walkthrough.steps.is_empty() {
            return invalid("walkthrough steps must not be empty");
        }
        for (index, step) in self.walkthrough.steps.iter().enumerate() {
            descriptive("walkthrough.step.text", &step.text)?;
            require_nonempty_refs(
                "evidence",
                &step.evidence_ids,
                &evidence_ids,
                &format!("walkthrough step {index}"),
            )?;
        }

        for (index, invariant) in self.invariants.iter().enumerate() {
            descriptive("invariant.text", &invariant.text)?;
            require_nonempty_refs(
                "evidence",
                &invariant.evidence_ids,
                &evidence_ids,
                &format!("invariant {index}"),
            )?;
        }

        let omitted = unique_ids(
            "omitted function",
            self.coverage.omitted_function_ids.iter(),
        )?;
        for fn_id in &omitted {
            if !roster_ids.contains(fn_id) {
                return invalid(format!(
                    "coverage references unknown omitted fnId {fn_id:?}"
                ));
            }
        }
        if self.coverage.mode == OrientationCoverageMode::FullSource && !omitted.is_empty() {
            return invalid("full-source coverage cannot omit functions");
        }
        if self.coverage.mode == OrientationCoverageMode::BoundedSource {
            let line_ranges = context.roster_line_ranges.ok_or_else(|| {
                OrientationValidationError::new(
                    "bounded-source validation requires verified roster line ranges",
                )
            })?;
            let mut covered_ranges = Vec::new();
            for fn_id in &roster_ids {
                let range = line_ranges.get(*fn_id).ok_or_else(|| {
                    OrientationValidationError::new(format!(
                        "bounded-source roster function {fn_id:?} has no verified line range"
                    ))
                })?;
                if !omitted.contains(fn_id) {
                    covered_ranges.push(*range);
                }
            }
            if covered_ranges.is_empty() {
                return invalid("bounded-source coverage has no included function source");
            }
            for evidence in &self.evidence {
                if !covered_ranges
                    .iter()
                    .any(|range| range[0] <= evidence.start_line && evidence.end_line <= range[1])
                {
                    return invalid(format!(
                        "evidence {} is outside every included bounded-source function span",
                        evidence.id
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Validate the stage-A trust boundary before its actor, flow, and evidence
/// IDs are exposed to any stage-B request. For bounded source, `roster_fn_ids`
/// must contain only functions whose exact source was included; their verified
/// spans are read from `roster_line_ranges`.
pub fn validate_orientation_skeleton(
    skeleton: &OrientationSkeleton,
    context: &OrientationValidationContext<'_>,
) -> Result<(), OrientationValidationError> {
    descriptive("purpose", &skeleton.purpose)?;

    let active_path = normalize_project_path(context.file_path)
        .ok_or_else(|| OrientationValidationError::new("active file path is unsafe"))?;

    if skeleton.actors.is_empty() {
        return invalid("actors must not be empty");
    }
    let actor_ids = unique_ids("actor", skeleton.actors.iter().map(|actor| &actor.id))?;
    for actor in &skeleton.actors {
        nonblank("actor.name", &actor.name)?;
        descriptive("actor.role", &actor.role)?;
    }

    if skeleton.evidence.is_empty() {
        return invalid("evidence must not be empty");
    }
    let evidence_ids = unique_ids(
        "evidence",
        skeleton.evidence.iter().map(|evidence| &evidence.id),
    )?;
    let source_lines = context.source.split('\n').count() as u32;
    for evidence in &skeleton.evidence {
        let evidence_path = normalize_project_path(&evidence.file_path).ok_or_else(|| {
            OrientationValidationError::new(format!(
                "evidence {} has an unsafe filePath",
                evidence.id
            ))
        })?;
        if evidence_path != active_path {
            return invalid(format!(
                "evidence {} points to {evidence_path:?}, not the active file",
                evidence.id
            ));
        }
        if evidence.start_line == 0
            || evidence.start_line > evidence.end_line
            || evidence.end_line > source_lines
        {
            return invalid(format!(
                "evidence {} has invalid line range {}..={} for {source_lines} lines",
                evidence.id, evidence.start_line, evidence.end_line
            ));
        }
    }

    if let Some(line_ranges) = context.roster_line_ranges {
        let mut allowed_ranges = Vec::new();
        for fn_id in context.roster_fn_ids {
            let range = line_ranges.get(fn_id).ok_or_else(|| {
                OrientationValidationError::new(format!(
                    "bounded-source skeleton function {fn_id:?} has no verified line range"
                ))
            })?;
            allowed_ranges.push(*range);
        }
        if allowed_ranges.is_empty() {
            return invalid("bounded-source skeleton has no included function source");
        }
        for evidence in &skeleton.evidence {
            if !allowed_ranges
                .iter()
                .any(|range| range[0] <= evidence.start_line && evidence.end_line <= range[1])
            {
                return invalid(format!(
                    "evidence {} is outside every included bounded-source function span",
                    evidence.id
                ));
            }
        }
    }

    if skeleton.core_flows.is_empty() {
        return invalid("coreFlows must not be empty");
    }
    unique_ids("flow", skeleton.core_flows.iter().map(|flow| &flow.id))?;
    for flow in &skeleton.core_flows {
        nonblank("flow.name", &flow.name)?;
        descriptive("flow.why", &flow.why)?;
        if flow.steps.is_empty() {
            return invalid(format!("core flow {} has no steps", flow.id));
        }
        for (index, step) in flow.steps.iter().enumerate() {
            nonblank("flow.step.via", &step.via)?;
            nonblank("flow.step.payload", &step.payload)?;
            descriptive("flow.step.why", &step.why)?;
            require_ref(
                "actor",
                &step.from_actor_id,
                &actor_ids,
                &format!("flow {} step {index} fromActorId", flow.id),
            )?;
            require_ref(
                "actor",
                &step.to_actor_id,
                &actor_ids,
                &format!("flow {} step {index} toActorId", flow.id),
            )?;
            require_nonempty_refs(
                "evidence",
                &step.evidence_ids,
                &evidence_ids,
                &format!("flow {} step {index}", flow.id),
            )?;
        }
    }

    for orientation_type in &skeleton.types {
        nonblank("type.name", &orientation_type.name)?;
        descriptive("type.meaning", &orientation_type.meaning)?;
        require_ref(
            "actor",
            &orientation_type.owner_actor_id,
            &actor_ids,
            &format!("type {} ownerActorId", orientation_type.name),
        )?;
    }

    descriptive("walkthrough.title", &skeleton.walkthrough.title)?;
    nonblank("walkthrough.input", &skeleton.walkthrough.input)?;
    if skeleton.walkthrough.steps.is_empty() {
        return invalid("walkthrough steps must not be empty");
    }
    for (index, step) in skeleton.walkthrough.steps.iter().enumerate() {
        descriptive("walkthrough.step.text", &step.text)?;
        require_nonempty_refs(
            "evidence",
            &step.evidence_ids,
            &evidence_ids,
            &format!("walkthrough step {index}"),
        )?;
    }

    for (index, invariant) in skeleton.invariants.iter().enumerate() {
        descriptive("invariant.text", &invariant.text)?;
        require_nonempty_refs(
            "evidence",
            &invariant.evidence_ids,
            &evidence_ids,
            &format!("invariant {index}"),
        )?;
    }

    Ok(())
}

/// Validate one stage-B response against both the backend-owned batch boundary
/// and the immutable IDs accepted at stage A.
pub fn validate_orientation_role_batch(
    batch: &OrientationRoleBatch,
    spec: &OrientationRoleBatchSpec,
    frozen: &OrientationSkeleton,
) -> Result<(), OrientationValidationError> {
    if spec.fn_ids.is_empty() {
        return invalid(format!("role batch {} has no functions", spec.index));
    }
    let spec_fn_ids = unique_ids("role batch function", spec.fn_ids.iter())?;
    let source_view_ids = unique_ids(
        "role batch source view",
        spec.source_views.iter().map(|view| match view {
            OrientationFunctionSourceView::Exact { fn_id, .. }
            | OrientationFunctionSourceView::SignatureOnly { fn_id, .. } => fn_id,
        }),
    )?;
    for view in &spec.source_views {
        nonblank("role batch source view", view.numbered_source())?;
        if !spec_fn_ids.contains(view.fn_id()) {
            return invalid(format!(
                "role batch {} source view references out-of-batch fnId {:?}",
                spec.index,
                view.fn_id()
            ));
        }
    }
    for fn_id in &spec_fn_ids {
        if !source_view_ids.contains(fn_id) {
            return invalid(format!(
                "role batch {} function {fn_id:?} has no source view",
                spec.index
            ));
        }
    }

    let actor_ids = unique_ids("frozen actor", frozen.actors.iter().map(|actor| &actor.id))?;
    let flow_ids = unique_ids("frozen flow", frozen.core_flows.iter().map(|flow| &flow.id))?;
    let evidence_ids = unique_ids(
        "frozen evidence",
        frozen.evidence.iter().map(|evidence| &evidence.id),
    )?;

    let role_fn_ids = unique_ids(
        "function role",
        batch.function_roles.iter().map(|role| &role.fn_id),
    )?;
    let mut role_lanes = BTreeMap::new();
    for role in &batch.function_roles {
        if !spec_fn_ids.contains(role.fn_id.as_str()) {
            return invalid(format!(
                "role batch {} references out-of-batch fnId {:?}",
                spec.index, role.fn_id
            ));
        }
        role_lanes.insert(role.fn_id.as_str(), role.lane);
        descriptive("functionRole.stage", &role.stage)?;
        descriptive("functionRole.why", &role.why)?;
        match role.lane {
            FunctionLane::Core if role.flow_ids.is_empty() => {
                return invalid(format!(
                    "core function {:?} must reference at least one flow",
                    role.fn_id
                ));
            }
            FunctionLane::Supporting if !role.flow_ids.is_empty() => {
                return invalid(format!(
                    "supporting function {:?} must not reference core flows",
                    role.fn_id
                ));
            }
            FunctionLane::Core | FunctionLane::Supporting => {}
        }
        require_refs(
            "flow",
            &role.flow_ids,
            &flow_ids,
            &format!("functionRole {}", role.fn_id),
        )?;
        require_refs(
            "actor",
            &role.receives_from_actor_ids,
            &actor_ids,
            &format!("functionRole {} receivesFromActorIds", role.fn_id),
        )?;
        require_refs(
            "actor",
            &role.sends_to_actor_ids,
            &actor_ids,
            &format!("functionRole {} sendsToActorIds", role.fn_id),
        )?;
        require_refs(
            "evidence",
            &role.evidence_ids,
            &evidence_ids,
            &format!("functionRole {}", role.fn_id),
        )?;
    }
    for fn_id in &spec_fn_ids {
        if !role_fn_ids.contains(fn_id) {
            return invalid(format!(
                "role batch {} function {fn_id:?} has no functionRole",
                spec.index
            ));
        }
    }

    for capability in &batch.supporting_capabilities {
        nonblank("supportingCapability.name", &capability.name)?;
        descriptive("supportingCapability.why", &capability.why)?;
        if capability.function_ids.is_empty() {
            return invalid(format!(
                "supporting capability {:?} has no functions",
                capability.name
            ));
        }
        for fn_id in &capability.function_ids {
            if !spec_fn_ids.contains(fn_id.as_str()) {
                return invalid(format!(
                    "supporting capability {:?} references out-of-batch fnId {fn_id:?}",
                    capability.name
                ));
            }
            let Some(lane) = role_lanes.get(fn_id.as_str()) else {
                return invalid(format!(
                    "supporting capability {:?} references fnId {fn_id:?} without a role",
                    capability.name
                ));
            };
            if *lane != FunctionLane::Supporting {
                return invalid(format!(
                    "core function {fn_id:?} also appears in supporting capabilities"
                ));
            }
        }
        require_nonempty_refs(
            "evidence",
            &capability.evidence_ids,
            &evidence_ids,
            &format!("supporting capability {}", capability.name),
        )?;
    }

    Ok(())
}

/// Combine validated stage outputs without guessing or repairing model data.
/// Function roles are placed in backend-roster order; supporting capabilities
/// retain batch order. The caller must still run `FileOrientationCard::validate`
/// with freshly verified source facts before caching or sending the card.
pub fn merge_orientation_card(
    skeleton: OrientationSkeleton,
    batches: Vec<(OrientationRoleBatchSpec, OrientationRoleBatch)>,
    backend: OrientationBackendFacts,
) -> Result<FileOrientationCard, OrientationValidationError> {
    let roster_ids = unique_ids("roster function", backend.roster_fn_ids.iter())?;
    let omitted_ids = unique_ids(
        "omitted function",
        backend.coverage.omitted_function_ids.iter(),
    )?;
    for fn_id in &omitted_ids {
        if !roster_ids.contains(fn_id) {
            return invalid(format!(
                "coverage references unknown omitted fnId {fn_id:?}"
            ));
        }
    }
    if backend.coverage.mode == OrientationCoverageMode::FullSource && !omitted_ids.is_empty() {
        return invalid("full-source coverage cannot omit functions");
    }

    let mut covered_spec_ids = BTreeSet::new();
    let mut roles_by_id = BTreeMap::new();
    let mut supporting_capabilities = Vec::new();
    for (spec, batch) in batches {
        validate_orientation_role_batch(&batch, &spec, &skeleton)?;
        for fn_id in &spec.fn_ids {
            if !roster_ids.contains(fn_id.as_str()) {
                return invalid(format!(
                    "role batch {} references fnId {fn_id:?} outside the backend roster",
                    spec.index
                ));
            }
            if !covered_spec_ids.insert(fn_id.clone()) {
                return invalid(format!(
                    "fnId {fn_id:?} appears in more than one role batch"
                ));
            }
        }
        for role in batch.function_roles {
            let fn_id = role.fn_id.clone();
            if roles_by_id.insert(fn_id.clone(), role).is_some() {
                return invalid(format!(
                    "fnId {fn_id:?} appears in more than one role batch"
                ));
            }
        }
        supporting_capabilities.extend(batch.supporting_capabilities);
    }

    for fn_id in &backend.roster_fn_ids {
        if !covered_spec_ids.contains(fn_id) {
            return invalid(format!(
                "roster function {fn_id:?} has no role batch coverage"
            ));
        }
    }
    let mut function_roles = Vec::with_capacity(backend.roster_fn_ids.len());
    for fn_id in &backend.roster_fn_ids {
        let role = roles_by_id.remove(fn_id).ok_or_else(|| {
            OrientationValidationError::new(format!(
                "roster function {fn_id:?} has no merged functionRole"
            ))
        })?;
        function_roles.push(role);
    }

    Ok(FileOrientationCard {
        schema_version: backend.schema_version,
        orientation_id: backend.orientation_id,
        file_path: backend.file_path,
        purpose: skeleton.purpose,
        actors: skeleton.actors,
        types: skeleton.types,
        core_flows: skeleton.core_flows,
        supporting_capabilities,
        function_roles,
        walkthrough: skeleton.walkthrough,
        invariants: skeleton.invariants,
        evidence: skeleton.evidence,
        coverage: backend.coverage,
    })
}

fn invalid<T>(message: impl Into<String>) -> Result<T, OrientationValidationError> {
    Err(OrientationValidationError::new(message))
}

fn nonblank(field: &str, value: &str) -> Result<(), OrientationValidationError> {
    if value.trim().is_empty() {
        invalid(format!("{field} must not be blank"))
    } else {
        Ok(())
    }
}

fn descriptive(field: &str, value: &str) -> Result<(), OrientationValidationError> {
    nonblank(field, value)?;
    if contains_unbound_direction(value) {
        invalid(format!(
            "{field} uses unbound upstream/downstream language; use actor IDs"
        ))
    } else {
        Ok(())
    }
}

fn contains_unbound_direction(value: &str) -> bool {
    if value.contains("上游") || value.contains("下游") {
        return true;
    }
    value
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .any(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "upstream" | "downstream"
            )
        })
}

fn unique_ids<'a>(
    kind: &str,
    values: impl IntoIterator<Item = &'a String>,
) -> Result<BTreeSet<&'a str>, OrientationValidationError> {
    let mut ids = BTreeSet::new();
    for value in values {
        nonblank(&format!("{kind} ID"), value)?;
        if !ids.insert(value.as_str()) {
            return invalid(format!("duplicate {kind} ID {value:?}"));
        }
    }
    Ok(ids)
}

fn require_ref(
    kind: &str,
    value: &str,
    allowed: &BTreeSet<&str>,
    owner: &str,
) -> Result<(), OrientationValidationError> {
    if allowed.contains(value) {
        Ok(())
    } else {
        invalid(format!("{owner} has dangling {kind} reference {value:?}"))
    }
}

fn require_refs(
    kind: &str,
    values: &[String],
    allowed: &BTreeSet<&str>,
    owner: &str,
) -> Result<(), OrientationValidationError> {
    for value in values {
        require_ref(kind, value, allowed, owner)?;
    }
    Ok(())
}

fn require_nonempty_refs(
    kind: &str,
    values: &[String],
    allowed: &BTreeSet<&str>,
    owner: &str,
) -> Result<(), OrientationValidationError> {
    if values.is_empty() {
        return invalid(format!("{owner} must reference at least one {kind} item"));
    }
    require_refs(kind, values, allowed, owner)
}

fn normalize_project_path(value: &str) -> Option<String> {
    let portable = value.replace('\\', "/");
    let path = Path::new(&portable);
    if path.is_absolute() {
        return None;
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => components.push(value.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

/// Exact inputs that define one orientation cache artifact. These fields are
/// explicit instead of inherited from `CacheStore`, so a concurrent settings
/// change cannot put an old response under a new provider/model identity.
#[derive(Debug, Clone, Copy)]
pub struct OrientationCacheIdentity<'a> {
    pub full_file_source: &'a str,
    pub relevant_graph_set_hash: &'a str,
    pub provider_base_url: &'a str,
    pub model: &'a str,
    pub prompt_version: &'a str,
    pub schema_version: u32,
}

impl OrientationCacheIdentity<'_> {
    pub fn key(&self) -> String {
        let schema_version = self.schema_version.to_le_bytes();
        stable_hash_parts([
            b"orientation-cache-v1".as_slice(),
            self.full_file_source.as_bytes(),
            self.relevant_graph_set_hash.as_bytes(),
            self.provider_base_url.as_bytes(),
            self.model.as_bytes(),
            self.prompt_version.as_bytes(),
            schema_version.as_slice(),
        ])
    }
}

/// Stable projection used by future child-artifact cache keys. Ordering of
/// set-like collections does not matter; flow-step order remains significant.
pub fn orientation_context_hash(card: &FileOrientationCard) -> String {
    let mut actors = card.actors.clone();
    actors.sort_by(|left, right| left.id.cmp(&right.id));

    let mut types = card.types.clone();
    types.sort_by(|left, right| {
        (&left.name, &left.owner_actor_id, &left.meaning).cmp(&(
            &right.name,
            &right.owner_actor_id,
            &right.meaning,
        ))
    });

    let mut flows = card.core_flows.clone();
    for flow in &mut flows {
        for step in &mut flow.steps {
            sort_dedup(&mut step.evidence_ids);
        }
    }
    flows.sort_by(|left, right| left.id.cmp(&right.id));

    let mut function_roles = card.function_roles.clone();
    for role in &mut function_roles {
        sort_dedup(&mut role.flow_ids);
        sort_dedup(&mut role.receives_from_actor_ids);
        sort_dedup(&mut role.consumes);
        sort_dedup(&mut role.sends_to_actor_ids);
        sort_dedup(&mut role.produces);
        sort_dedup(&mut role.evidence_ids);
    }
    function_roles.sort_by(|left, right| left.fn_id.cmp(&right.fn_id));

    let mut evidence = card.evidence.clone();
    for item in &mut evidence {
        if let Some(path) = normalize_project_path(&item.file_path) {
            item.file_path = path;
        }
    }
    evidence.sort_by(|left, right| left.id.cmp(&right.id));

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ContextProjection {
        actors: Vec<OrientationActor>,
        types: Vec<OrientationType>,
        flows: Vec<OrientationFlow>,
        function_roles: Vec<FunctionRole>,
        evidence: Vec<CodeEvidenceRef>,
    }

    let projection = ContextProjection {
        actors,
        types,
        flows,
        function_roles,
        evidence,
    };
    let bytes = serde_json::to_vec(&projection)
        .expect("orientation context projection contains only serializable fields");
    stable_hash_parts(std::iter::once(bytes.as_slice()))
}

fn sort_dedup(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn stable_hash_parts<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hash = FNV_OFFSET;
    for part in parts {
        for byte in (part.len() as u64).to_le_bytes().iter().chain(part.iter()) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_store::CacheStore;

    fn validation_source() -> &'static str {
        "fn fetch() {\n    send();\n}\nfn helper() {}\n"
    }

    fn roster() -> Vec<String> {
        vec!["fetch".into(), "helper".into()]
    }

    fn validation_context<'a>(roster: &'a [String]) -> OrientationValidationContext<'a> {
        OrientationValidationContext {
            file_path: "src/a.rs",
            source: validation_source(),
            roster_fn_ids: roster,
            roster_line_ranges: None,
        }
    }

    fn identity<'a>(source: &'a str) -> OrientationCacheIdentity<'a> {
        OrientationCacheIdentity {
            full_file_source: source,
            relevant_graph_set_hash: "graph-v1",
            provider_base_url: "https://provider.test/v1",
            model: "model-v1",
            prompt_version: ORIENTATION_PROMPT_VERSION,
            schema_version: ORIENTATION_SCHEMA_VERSION,
        }
    }

    fn valid_card(orientation_id: String) -> FileOrientationCard {
        FileOrientationCard {
            schema_version: ORIENTATION_SCHEMA_VERSION,
            orientation_id,
            file_path: "src/a.rs".into(),
            purpose: "Receive a request and deliver a response.".into(),
            actors: vec![
                OrientationActor {
                    id: "caller".into(),
                    name: "Caller".into(),
                    role: "Starts the request.".into(),
                    boundary: ActorBoundary::Project,
                },
                OrientationActor {
                    id: "worker".into(),
                    name: "Worker".into(),
                    role: "Handles the request.".into(),
                    boundary: ActorBoundary::InsideFile,
                },
            ],
            types: vec![OrientationType {
                name: "Request".into(),
                owner_actor_id: "caller".into(),
                meaning: "One unit of requested work.".into(),
            }],
            core_flows: vec![OrientationFlow {
                id: "request-flow".into(),
                name: "Request delivery".into(),
                kind: OrientationFlowKind::Request,
                why: "The worker needs a concrete request.".into(),
                steps: vec![OrientationFlowStep {
                    from_actor_id: "caller".into(),
                    via: "fetch".into(),
                    payload: "Request".into(),
                    to_actor_id: "worker".into(),
                    why: "Transfers ownership of the work.".into(),
                    evidence_ids: vec!["E1".into()],
                }],
            }],
            supporting_capabilities: vec![SupportingCapability {
                name: "Local helper".into(),
                why: "Keeps the core operation small.".into(),
                function_ids: vec!["helper".into()],
                evidence_ids: vec!["E2".into()],
            }],
            function_roles: vec![
                FunctionRole {
                    fn_id: "fetch".into(),
                    lane: FunctionLane::Core,
                    flow_ids: vec!["request-flow".into()],
                    stage: "dispatch".into(),
                    receives_from_actor_ids: vec!["caller".into()],
                    consumes: vec!["Request".into()],
                    sends_to_actor_ids: vec!["worker".into()],
                    produces: vec!["work".into()],
                    why: "Moves the request into the worker.".into(),
                    evidence_ids: vec!["E1".into()],
                },
                FunctionRole {
                    fn_id: "helper".into(),
                    lane: FunctionLane::Supporting,
                    flow_ids: Vec::new(),
                    stage: "utility".into(),
                    receives_from_actor_ids: vec!["worker".into()],
                    consumes: vec!["work".into()],
                    sends_to_actor_ids: vec!["worker".into()],
                    produces: vec!["prepared work".into()],
                    why: "Prepares local state.".into(),
                    evidence_ids: vec!["E2".into()],
                },
            ],
            walkthrough: OrientationWalkthrough {
                title: "One request".into(),
                input: "request-1".into(),
                steps: vec![WalkthroughStep {
                    text: "Caller invokes fetch with request-1.".into(),
                    evidence_ids: vec!["E1".into()],
                }],
            },
            invariants: vec![OrientationInvariant {
                text: "The request is handled inside this file.".into(),
                evidence_ids: vec!["E1".into()],
            }],
            evidence: vec![
                CodeEvidenceRef {
                    id: "E1".into(),
                    file_path: "src/a.rs".into(),
                    start_line: 1,
                    end_line: 3,
                    symbol: Some("fetch".into()),
                },
                CodeEvidenceRef {
                    id: "E2".into(),
                    file_path: "src/a.rs".into(),
                    start_line: 4,
                    end_line: 4,
                    symbol: Some("helper".into()),
                },
            ],
            coverage: OrientationCoverage {
                mode: OrientationCoverageMode::FullSource,
                omitted_function_ids: Vec::new(),
            },
        }
    }

    fn valid_skeleton() -> OrientationSkeleton {
        let card = valid_card("unused-backend-id".into());
        OrientationSkeleton {
            purpose: card.purpose,
            actors: card.actors,
            types: card.types,
            core_flows: card.core_flows,
            walkthrough: card.walkthrough,
            invariants: card.invariants,
            evidence: card.evidence,
        }
    }

    fn role_batch_spec(index: usize, fn_ids: &[&str]) -> OrientationRoleBatchSpec {
        OrientationRoleBatchSpec {
            index,
            fn_ids: fn_ids.iter().map(|fn_id| (*fn_id).to_string()).collect(),
            source_views: fn_ids
                .iter()
                .map(|fn_id| OrientationFunctionSourceView::Exact {
                    fn_id: (*fn_id).to_string(),
                    numbered_source: format!("1 | fn {fn_id}() {{}}"),
                })
                .collect(),
        }
    }

    fn valid_role_batch(fn_ids: &[&str]) -> OrientationRoleBatch {
        let card = valid_card("unused-backend-id".into());
        let allowed = fn_ids.iter().copied().collect::<BTreeSet<_>>();
        OrientationRoleBatch {
            function_roles: card
                .function_roles
                .into_iter()
                .filter(|role| allowed.contains(role.fn_id.as_str()))
                .collect(),
            supporting_capabilities: card
                .supporting_capabilities
                .into_iter()
                .filter_map(|mut capability| {
                    capability
                        .function_ids
                        .retain(|fn_id| allowed.contains(fn_id.as_str()));
                    (!capability.function_ids.is_empty()).then_some(capability)
                })
                .collect(),
        }
    }

    fn backend_facts(roster_fn_ids: Vec<String>) -> OrientationBackendFacts {
        OrientationBackendFacts {
            schema_version: ORIENTATION_SCHEMA_VERSION,
            orientation_id: "orientation-merged".into(),
            file_path: "src/a.rs".into(),
            coverage: OrientationCoverage {
                mode: OrientationCoverageMode::FullSource,
                omitted_function_ids: Vec::new(),
            },
            roster_fn_ids,
        }
    }

    #[test]
    fn valid_card_round_trips_and_validates() {
        let roster = roster();
        let card = valid_card("orientation-1".into());

        card.validate(&validation_context(&roster)).unwrap();
        let json = serde_json::to_string(&card).unwrap();
        assert!(json.contains("\"schemaVersion\""));
        assert!(json.contains("\"functionRoles\""));
        assert_eq!(
            serde_json::from_str::<FileOrientationCard>(&json).unwrap(),
            card
        );
    }

    #[test]
    fn validator_rejects_duplicate_actor_flow_and_evidence_ids() {
        let roster = roster();
        let context = validation_context(&roster);

        let mut duplicate_actor = valid_card("orientation-1".into());
        duplicate_actor
            .actors
            .push(duplicate_actor.actors[0].clone());
        assert!(duplicate_actor.validate(&context).is_err());

        let mut duplicate_flow = valid_card("orientation-1".into());
        duplicate_flow
            .core_flows
            .push(duplicate_flow.core_flows[0].clone());
        assert!(duplicate_flow.validate(&context).is_err());

        let mut duplicate_evidence = valid_card("orientation-1".into());
        duplicate_evidence
            .evidence
            .push(duplicate_evidence.evidence[0].clone());
        assert!(duplicate_evidence.validate(&context).is_err());
    }

    #[test]
    fn validator_rejects_dangling_actor_flow_and_evidence_references() {
        let roster = roster();
        let context = validation_context(&roster);

        let mut actor = valid_card("orientation-1".into());
        actor.core_flows[0].steps[0].to_actor_id = "missing".into();
        assert!(actor.validate(&context).is_err());

        let mut flow = valid_card("orientation-1".into());
        flow.function_roles[0].flow_ids = vec!["missing".into()];
        assert!(flow.validate(&context).is_err());

        let mut evidence = valid_card("orientation-1".into());
        evidence.invariants[0].evidence_ids = vec!["missing".into()];
        assert!(evidence.validate(&context).is_err());
    }

    #[test]
    fn validator_rejects_wrong_paths_and_line_ranges() {
        let roster = roster();
        let context = validation_context(&roster);

        let mut wrong_path = valid_card("orientation-1".into());
        wrong_path.evidence[0].file_path = "src/other.rs".into();
        assert!(wrong_path.validate(&context).is_err());

        let mut zero = valid_card("orientation-1".into());
        zero.evidence[0].start_line = 0;
        assert!(zero.validate(&context).is_err());

        let mut reversed = valid_card("orientation-1".into());
        reversed.evidence[0].start_line = 3;
        reversed.evidence[0].end_line = 2;
        assert!(reversed.validate(&context).is_err());

        let mut beyond_source = valid_card("orientation-1".into());
        beyond_source.evidence[0].end_line = 6;
        assert!(beyond_source.validate(&context).is_err());
    }

    #[test]
    fn validator_rejects_unknown_unassigned_or_overlapping_function_roles() {
        let roster = roster();
        let context = validation_context(&roster);

        let mut unknown = valid_card("orientation-1".into());
        unknown.function_roles[0].fn_id = "missing".into();
        assert!(unknown.validate(&context).is_err());

        let mut unassigned = valid_card("orientation-1".into());
        unassigned.function_roles.pop();
        assert!(unassigned.validate(&context).is_err());

        let mut overlap = valid_card("orientation-1".into());
        let mut duplicate = overlap.function_roles[0].clone();
        duplicate.lane = FunctionLane::Supporting;
        overlap.function_roles.push(duplicate);
        assert!(overlap.validate(&context).is_err());
    }

    #[test]
    fn validator_rejects_empty_core_flow_evidence_or_walkthrough() {
        let roster = roster();
        let context = validation_context(&roster);

        let mut empty_flow = valid_card("orientation-1".into());
        empty_flow.core_flows[0].steps.clear();
        assert!(empty_flow.validate(&context).is_err());

        let mut core_role_without_flow = valid_card("orientation-1".into());
        core_role_without_flow.function_roles[0].flow_ids.clear();
        assert!(core_role_without_flow.validate(&context).is_err());

        let mut step_without_evidence = valid_card("orientation-1".into());
        step_without_evidence.core_flows[0].steps[0]
            .evidence_ids
            .clear();
        assert!(step_without_evidence.validate(&context).is_err());

        let mut empty_walkthrough = valid_card("orientation-1".into());
        empty_walkthrough.walkthrough.steps.clear();
        assert!(empty_walkthrough.validate(&context).is_err());
    }

    #[test]
    fn bounded_validator_accepts_only_evidence_from_non_omitted_function_spans() {
        let roster = roster();
        let line_ranges = BTreeMap::from([
            ("fetch".to_string(), [1, 3]),
            ("helper".to_string(), [4, 4]),
        ]);
        let context = OrientationValidationContext {
            file_path: "src/a.rs",
            source: validation_source(),
            roster_fn_ids: &roster,
            roster_line_ranges: Some(&line_ranges),
        };

        let mut invalid = valid_card("orientation-1".into());
        invalid.coverage = OrientationCoverage {
            mode: OrientationCoverageMode::BoundedSource,
            omitted_function_ids: vec!["helper".into()],
        };
        assert!(invalid.validate(&context).is_err());

        let mut valid = invalid;
        valid.supporting_capabilities.clear();
        valid.function_roles[1].evidence_ids.clear();
        valid.evidence.retain(|evidence| evidence.id == "E1");
        assert!(valid.validate(&context).is_ok());
    }

    #[test]
    fn validator_rejects_unbound_upstream_or_downstream_language() {
        let roster = roster();
        let context = validation_context(&roster);

        let mut chinese = valid_card("orientation-1".into());
        chinese.purpose = "把结果交给下游".into();
        assert!(chinese.validate(&context).is_err());

        let mut english = valid_card("orientation-1".into());
        english.core_flows[0].why = "Needed by the downstream consumer.".into();
        assert!(english.validate(&context).is_err());
    }

    #[test]
    fn skeleton_validator_rejects_dangling_actor_evidence_and_empty_core_flows() {
        let roster = roster();
        let context = validation_context(&roster);
        validate_orientation_skeleton(&valid_skeleton(), &context).unwrap();

        let mut dangling_actor = valid_skeleton();
        dangling_actor.core_flows[0].steps[0].to_actor_id = "missing".into();
        assert!(validate_orientation_skeleton(&dangling_actor, &context).is_err());

        let mut dangling_evidence = valid_skeleton();
        dangling_evidence.walkthrough.steps[0].evidence_ids = vec!["missing".into()];
        assert!(validate_orientation_skeleton(&dangling_evidence, &context).is_err());

        let mut no_core_flows = valid_skeleton();
        no_core_flows.core_flows.clear();
        assert!(validate_orientation_skeleton(&no_core_flows, &context).is_err());

        let mut empty_core_flow = valid_skeleton();
        empty_core_flow.core_flows[0].steps.clear();
        assert!(validate_orientation_skeleton(&empty_core_flow, &context).is_err());
    }

    #[test]
    fn skeleton_validator_enforces_bounded_source_evidence_ranges() {
        let included = vec!["fetch".to_string()];
        let line_ranges = BTreeMap::from([
            ("fetch".to_string(), [1, 3]),
            ("helper".to_string(), [4, 4]),
        ]);
        let context = OrientationValidationContext {
            file_path: "src/a.rs",
            source: validation_source(),
            roster_fn_ids: &included,
            roster_line_ranges: Some(&line_ranges),
        };

        let mut outside = valid_skeleton();
        assert!(validate_orientation_skeleton(&outside, &context).is_err());

        outside.evidence.retain(|evidence| evidence.id == "E1");
        validate_orientation_skeleton(&outside, &context).unwrap();
    }

    #[test]
    fn role_batch_validator_rejects_missing_duplicate_and_out_of_batch_functions() {
        let frozen = valid_skeleton();
        let spec = role_batch_spec(0, &["fetch", "helper"]);
        validate_orientation_role_batch(&valid_role_batch(&["fetch", "helper"]), &spec, &frozen)
            .unwrap();

        let mut missing = valid_role_batch(&["fetch", "helper"]);
        missing.function_roles.pop();
        assert!(validate_orientation_role_batch(&missing, &spec, &frozen).is_err());

        let mut duplicate = valid_role_batch(&["fetch", "helper"]);
        duplicate
            .function_roles
            .push(duplicate.function_roles[0].clone());
        assert!(validate_orientation_role_batch(&duplicate, &spec, &frozen).is_err());

        let mut outside = valid_role_batch(&["fetch", "helper"]);
        outside.function_roles[0].fn_id = "outside".into();
        assert!(validate_orientation_role_batch(&outside, &spec, &frozen).is_err());
    }

    #[test]
    fn role_batch_validator_requires_one_source_view_per_batch_function() {
        let frozen = valid_skeleton();
        let batch = valid_role_batch(&["helper"]);

        let mut missing = role_batch_spec(0, &["helper"]);
        missing.source_views.clear();
        assert!(validate_orientation_role_batch(&batch, &missing, &frozen).is_err());

        let mut outside = role_batch_spec(0, &["helper"]);
        outside.source_views[0] = OrientationFunctionSourceView::Exact {
            fn_id: "outside".into(),
            numbered_source: "1 | fn outside() {}".into(),
        };
        assert!(validate_orientation_role_batch(&batch, &outside, &frozen).is_err());

        let signature_only = OrientationRoleBatchSpec {
            index: 0,
            fn_ids: vec!["helper".into()],
            source_views: vec![OrientationFunctionSourceView::SignatureOnly {
                fn_id: "helper".into(),
                numbered_signature: "4 | fn helper()".into(),
            }],
        };
        validate_orientation_role_batch(&batch, &signature_only, &frozen).unwrap();
    }

    #[test]
    fn role_batch_validator_rejects_ids_outside_the_frozen_skeleton() {
        let frozen = valid_skeleton();
        let fetch_spec = role_batch_spec(0, &["fetch"]);

        let mut actor = valid_role_batch(&["fetch"]);
        actor.function_roles[0].receives_from_actor_ids = vec!["missing".into()];
        assert!(validate_orientation_role_batch(&actor, &fetch_spec, &frozen).is_err());

        let mut flow = valid_role_batch(&["fetch"]);
        flow.function_roles[0].flow_ids = vec!["missing".into()];
        assert!(validate_orientation_role_batch(&flow, &fetch_spec, &frozen).is_err());

        let helper_spec = role_batch_spec(1, &["helper"]);
        let mut evidence = valid_role_batch(&["helper"]);
        evidence.supporting_capabilities[0].evidence_ids = vec!["missing".into()];
        assert!(validate_orientation_role_batch(&evidence, &helper_spec, &frozen).is_err());

        let mut supporting_flow = valid_role_batch(&["helper"]);
        supporting_flow.function_roles[0].flow_ids = vec!["request-flow".into()];
        assert!(validate_orientation_role_batch(&supporting_flow, &helper_spec, &frozen).is_err());
    }

    #[test]
    fn merge_orders_multiple_batches_by_roster_and_preserves_final_validation() {
        let frozen = valid_skeleton();
        let batches = vec![
            (role_batch_spec(0, &["fetch"]), valid_role_batch(&["fetch"])),
            (
                role_batch_spec(1, &["helper"]),
                valid_role_batch(&["helper"]),
            ),
        ];
        let roster = vec!["helper".to_string(), "fetch".to_string()];

        let card = merge_orientation_card(frozen, batches, backend_facts(roster.clone())).unwrap();

        assert_eq!(
            card.function_roles
                .iter()
                .map(|role| role.fn_id.as_str())
                .collect::<Vec<_>>(),
            vec!["helper", "fetch"]
        );
        assert_eq!(card.supporting_capabilities[0].name, "Local helper");
        card.validate(&validation_context(&roster)).unwrap();
    }

    #[test]
    fn merge_rejects_missing_or_duplicate_cross_batch_roster_coverage() {
        let frozen = valid_skeleton();
        let roster = roster();

        let missing = vec![(role_batch_spec(0, &["fetch"]), valid_role_batch(&["fetch"]))];
        assert!(
            merge_orientation_card(frozen.clone(), missing, backend_facts(roster.clone())).is_err()
        );

        let duplicate = vec![
            (role_batch_spec(0, &["fetch"]), valid_role_batch(&["fetch"])),
            (role_batch_spec(1, &["fetch"]), valid_role_batch(&["fetch"])),
        ];
        assert!(merge_orientation_card(frozen, duplicate, backend_facts(roster)).is_err());
    }

    #[test]
    fn context_hash_is_stable_across_set_like_ordering() {
        let card = valid_card("orientation-1".into());
        let mut reordered = card.clone();
        reordered.actors.reverse();
        reordered.function_roles.reverse();
        reordered.evidence.reverse();
        reordered.function_roles[0].consumes.reverse();

        assert_eq!(
            orientation_context_hash(&card),
            orientation_context_hash(&reordered)
        );
    }

    #[test]
    fn orientation_cache_round_trips_and_every_identity_field_causes_a_miss() {
        let dir = tempdir_guard::TempDir::new();
        let cache = CacheStore::new(dir.path(), "capsule-model", "capsule-prompt");
        let source = validation_source();
        let base = identity(source);
        let roster = roster();
        let context = validation_context(&roster);
        let card = valid_card(base.key());

        assert!(cache.get_orientation(&base, &context).is_none());
        cache.put_orientation(&base, &context, &card).unwrap();
        assert_eq!(cache.get_orientation(&base, &context), Some(card));

        let changed = [
            OrientationCacheIdentity {
                full_file_source: "fn changed() {}\n",
                ..base
            },
            OrientationCacheIdentity {
                relevant_graph_set_hash: "graph-v2",
                ..base
            },
            OrientationCacheIdentity {
                provider_base_url: "https://other.test/v1",
                ..base
            },
            OrientationCacheIdentity {
                model: "model-v2",
                ..base
            },
            OrientationCacheIdentity {
                prompt_version: "orientation-p-other",
                ..base
            },
            OrientationCacheIdentity {
                schema_version: ORIENTATION_SCHEMA_VERSION + 1,
                ..base
            },
        ];
        for alternate in changed {
            assert_ne!(alternate.key(), base.key());
            assert!(cache.get_orientation(&alternate, &context).is_none());
        }
    }

    #[test]
    fn orientation_cache_rejects_invalid_cards_and_leaves_source_bytes_and_mtime_untouched() {
        let dir = tempdir_guard::TempDir::new();
        let source_path = dir.path().join("src").join("a.rs");
        std::fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        std::fs::write(&source_path, validation_source()).unwrap();
        let before_bytes = std::fs::read(&source_path).unwrap();
        let before_mtime = std::fs::metadata(&source_path).unwrap().modified().unwrap();

        let cache = CacheStore::new(dir.path(), "capsule-model", "capsule-prompt");
        let base = identity(validation_source());
        let roster = roster();
        let context = validation_context(&roster);
        let mut invalid = valid_card(base.key());
        invalid.core_flows[0].steps[0].evidence_ids.clear();

        assert!(cache.put_orientation(&base, &context, &invalid).is_err());

        let valid = valid_card(base.key());
        cache.put_orientation(&base, &context, &valid).unwrap();
        let cache_path = dir
            .path()
            .join(".fluid")
            .join("orientations")
            .join(format!("{}.json", base.key()));
        assert!(cache_path.is_file());
        assert_eq!(cache.get_orientation(&base, &context), Some(valid.clone()));

        let mut poisoned = valid;
        poisoned.core_flows[0].steps.clear();
        std::fs::write(&cache_path, serde_json::to_vec(&poisoned).unwrap()).unwrap();
        assert!(
            cache.get_orientation(&base, &context).is_none(),
            "an invalid disk artifact must be treated as a miss"
        );
        assert_eq!(std::fs::read(&source_path).unwrap(), before_bytes);
        assert_eq!(
            std::fs::metadata(&source_path).unwrap().modified().unwrap(),
            before_mtime
        );
    }

    mod tempdir_guard {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                let unique = format!(
                    "fluid-orientation-test-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                );
                let path = std::env::temp_dir().join(unique);
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
