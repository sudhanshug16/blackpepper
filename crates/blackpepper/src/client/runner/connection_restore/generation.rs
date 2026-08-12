use crate::core::HostId;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct GenerationGate {
    next: BTreeMap<HostId, u64>,
    pub(super) active: BTreeMap<HostId, (u64, uuid::Uuid)>,
}

impl GenerationGate {
    pub(super) fn begin(&mut self, host_id: HostId) -> (u64, uuid::Uuid) {
        let generation = self.next.entry(host_id).or_default();
        *generation += 1;
        let token = uuid::Uuid::new_v4();
        self.active.insert(host_id, (*generation, token));
        (*generation, token)
    }

    pub(super) fn is_current(&self, host_id: HostId, generation: u64, token: uuid::Uuid) -> bool {
        self.active.get(&host_id) == Some(&(generation, token))
    }

    pub(super) fn invalidate(&mut self, host_id: HostId) {
        self.active.remove(&host_id);
    }

    pub(super) fn finish(&mut self, host_id: HostId, generation: u64, token: uuid::Uuid) -> bool {
        if !self.is_current(host_id, generation, token) {
            return false;
        }
        self.active.remove(&host_id);
        true
    }
}
