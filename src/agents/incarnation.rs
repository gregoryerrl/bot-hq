//! Lifetime of one installed participant runtime (including its CLI retries).
//!
//! Killing the process is asynchronous. This fence separately revokes its
//! host-side writers and waits for writes already admitted to finish before a
//! replacement can be published under the same participant id/slug.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{RwLock, RwLockReadGuard};

#[derive(Debug)]
pub struct Incarnation {
    id: String,
    active: AtomicBool,
    writers: Arc<RwLock<()>>,
    helpers: std::sync::Mutex<HelperChildren>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HelperTicket { epoch: u64, generation: u64 }
type StopHelper = Box<dyn Fn(&str) + Send + Sync>;
#[derive(Default)]
struct HelperChildren {
    generation: u64,
    closed: bool,
    ended_epoch: u64,
    children: std::collections::HashMap<String, (u64, StopHelper)>,
}
impl std::fmt::Debug for HelperChildren {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelperChildren").field("closed",&self.closed).field("count",&self.children.len()).finish()
    }
}

/// Held by the host job until helper cleanup is acknowledged. Dropping a job
/// cannot silently detach a child from its parent.
pub(crate) struct HelperRegistration { owner: Arc<Incarnation>, id: String }
impl HelperRegistration {
    pub(crate) fn release(self) {
        self.owner.helpers.lock().unwrap_or_else(|e|e.into_inner()).children.remove(&self.id);
    }
}
impl Drop for HelperRegistration {
    fn drop(&mut self) {
        let child=self.owner.helpers.lock().unwrap_or_else(|e|e.into_inner()).children.remove(&self.id);
        if let Some((_,stop))=child { stop("helper owner released"); }
    }
}

impl Incarnation {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            active: AtomicBool::new(true),
            writers: Arc::new(RwLock::new(())),
            helpers: std::sync::Mutex::new(HelperChildren::default()),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) fn helper_ticket(&self, epoch: u64) -> Option<HelperTicket> {
        let state=self.helpers.lock().unwrap_or_else(|e|e.into_inner());
        (self.is_active() && !state.closed && epoch>state.ended_epoch)
            .then_some(HelperTicket {epoch,generation:state.generation})
    }

    pub(crate) fn helper_ticket_valid(&self, ticket: HelperTicket) -> bool {
        let state=self.helpers.lock().unwrap_or_else(|e|e.into_inner());
        self.is_active() && !state.closed && ticket.epoch>state.ended_epoch && ticket.generation==state.generation
    }

    pub(crate) fn attach_helper(self: &Arc<Self>, id: String, ticket: HelperTicket, stop: StopHelper) -> Option<HelperRegistration> {
        let mut state=self.helpers.lock().unwrap_or_else(|e|e.into_inner());
        if !self.is_active() || state.closed || ticket.epoch<=state.ended_epoch || ticket.generation!=state.generation || state.children.contains_key(&id) {
            drop(state); stop("parent stopped before helper registration"); return None;
        }
        state.children.insert(id.clone(),(ticket.epoch,stop));
        Some(HelperRegistration {owner:Arc::clone(self),id})
    }

    pub(crate) fn stop_helpers(&self, reason: &str, close: bool) {
        let children={
            let mut state=self.helpers.lock().unwrap_or_else(|e|e.into_inner());
            state.generation=state.generation.wrapping_add(1);
            state.closed |= close;
            std::mem::take(&mut state.children)
        };
        for (_,(_,stop)) in children {stop(reason);}
    }

    pub(crate) fn end_helper_epoch(&self, epoch: u64) {
        let children={
            let mut state=self.helpers.lock().unwrap_or_else(|e|e.into_inner());
            state.ended_epoch=state.ended_epoch.max(epoch);
            let ids: Vec<_>=state.children.iter().filter(|(_, (owned,_))|*owned==epoch).map(|(id,_)|id.clone()).collect();
            ids.into_iter().filter_map(|id|state.children.remove(&id)).collect::<Vec<_>>()
        };
        for (_,stop) in children {stop("parent turn ended");}
    }

    /// Keep the lease through every await that can mutate host state. A check
    /// followed by an unguarded async write would still race retirement.
    pub async fn enter(&self) -> Option<RwLockReadGuard<'_, ()>> {
        if !self.is_active() {
            return None;
        }
        let lease = self.writers.read().await;
        self.is_active().then_some(lease)
    }

    /// A queued control keeps its lease through the eventual delivery, even
    /// when selection and execution live in different functions.
    pub async fn enter_owned(&self) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
        if !self.is_active() {
            return None;
        }
        let lease = Arc::clone(&self.writers).read_owned().await;
        self.is_active().then_some(lease)
    }

    /// Revoke new writers immediately, then drain admitted ones. Cancellation
    /// cannot reactivate the old runtime; a retry can finish draining it.
    pub async fn retire(&self) {
        self.active.store(false, Ordering::Release);
        self.stop_helpers("parent runtime retired",true);
        let _drained = self.writers.write().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn helper_tickets_lose_to_interrupt_completion_and_retirement() {
        use std::sync::atomic::AtomicUsize;
        let owner=Incarnation::new(); let count=Arc::new(AtomicUsize::new(0));
        let stop=|| {let c=count.clone(); Box::new(move |_: &str|{c.fetch_add(1,Ordering::SeqCst);}) as StopHelper};
        let ticket=owner.helper_ticket(1).unwrap();
        let _child=owner.attach_helper("child".into(),ticket,stop()).unwrap();
        owner.stop_helpers("paused",false);
        assert_eq!(count.load(Ordering::SeqCst),1);
        assert!(owner.attach_helper("late".into(),ticket,stop()).is_none());
        let second=owner.helper_ticket(2).unwrap();
        let _new=owner.attach_helper("next".into(),second,stop()).unwrap();
        owner.end_helper_epoch(1);
        assert_eq!(count.load(Ordering::SeqCst),2,"old completion must not kill newer helper");
        owner.end_helper_epoch(2);
        assert!(owner.helper_ticket(2).is_none());
        assert_eq!(count.load(Ordering::SeqCst),3);
        let waiting=owner.helper_ticket(3).unwrap(); owner.retire().await;
        assert!(owner.attach_helper("retired".into(),waiting,stop()).is_none());
        assert!(owner.helper_ticket(4).is_none());
    }

    #[tokio::test]
    async fn retirement_waits_for_an_admitted_async_write_and_refuses_new_ones() {
        let incarnation = Incarnation::new();
        let lease = incarnation.enter().await.unwrap();
        let retired = incarnation.clone();
        let mut task = tokio::spawn(async move { retired.retire().await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut task)
                .await
                .is_err()
        );
        assert!(!incarnation.is_active());
        assert!(incarnation.enter().await.is_none());
        drop(lease);
        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(incarnation.enter().await.is_none());
        let replacement = Incarnation::new();
        assert_ne!(replacement.id(), incarnation.id());
        assert!(replacement.enter().await.is_some());
    }

    #[tokio::test]
    async fn a_cancelled_retirement_never_reactivates_the_old_runtime() {
        let incarnation = Incarnation::new();
        let lease = incarnation.enter().await.unwrap();
        let retired = incarnation.clone();
        let mut task = tokio::spawn(async move { retired.retire().await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut task)
                .await
                .is_err()
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(lease);
        assert!(incarnation.enter().await.is_none());
        incarnation.retire().await;
    }
}
