//! Durable, UI-independent host state and helper protocol.

mod ids;
mod paths;
mod protocol;
mod records;
mod registry;
mod repository;
mod singleton;

pub use ids::{AgentRunId, HostId, PaneId, RepositoryId, SessionId, WorkspaceId};
pub use paths::{CorePaths, PathError};
pub use protocol::{
    serve_json_lines, serve_json_lines_with_extension, AgentProcessObservation, AgentRunBinding,
    FailureCode, HelperRequest, HelperResponse, HostAgentRun, HostAgentSnapshot, HostAgentUpdate,
    HostPeriodicRefresh, HostServicePayload, ProtocolError, ProtocolExtension, ProtocolFailure,
    PullRequestState, PullRequestSummary, RepositoryInspection, RequestOperation, ResponsePayload,
    ResponseResult, WorkspaceOverview, WorktrunkMutationOutcome, PROTOCOL_VERSION,
};
pub use records::{
    HostRecord, HostTransport, RegistrySnapshot, SessionBackend, SessionRecord, SessionState,
    WorkspaceRecord, WorkspaceSetup,
};
pub(crate) use registry::WorktrunkRemovalIntent;
pub use registry::{HostRegistry, RegistryError};
pub use repository::{
    canonicalize_remote_url, GroupingPolicy, RepositoryIdentity, RepositoryIdentityError,
};
pub use singleton::{SingletonLock, SingletonLockError};
