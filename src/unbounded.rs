use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::Notify;
use tokio::sync::RwLock as AsyncRwLock;

pub struct UnboundedNapMap<K, V>
where
    K: Eq + Hash + Clone + Debug,
    V: Clone + Debug,
{
    map: Arc<AsyncRwLock<HashMap<K, V>>>,
    notifiers: Arc<AsyncMutex<HashMap<K, Arc<Notify>>>>,
}

/// Creates an unbounded napmap for communicating between asynchronous tasks.
///
/// **Note** that the amount of available system memory is an implicit bound to
/// the map. Using an `unbounded` map has the ability of causing the
/// process to run out of memory. In this case, the process will be aborted.
pub fn unbounded<K, V>() -> UnboundedNapMap<K, V>
where
    K: Eq + Hash + Clone + Debug,
    V: Clone + Debug,
{
    UnboundedNapMap::new()
}

impl<K, V> UnboundedNapMap<K, V>
where
    K: Eq + Hash + Clone + Debug,
    V: Clone + Debug,
{
    pub fn new() -> Self {
        Self {
            map: Arc::new(AsyncRwLock::new(HashMap::new())),
            notifiers: Arc::new(AsyncMutex::new(HashMap::new())),
        }
    }

    #[tracing::instrument(level = tracing::Level::TRACE, skip(self, v))]
    pub async fn insert(&self, k: K, v: V) {
        tracing::trace!("Insert");
        self.map.write().await.insert(k.clone(), v);
        if let Some(notify) = self.notifiers.lock().await.remove(&k) {
            notify.notify_waiters();
            tracing::trace!("Notified all waiting tasks");
        }
    }

    #[tracing::instrument(level = tracing::Level::TRACE, skip(self))]
    pub async fn get(&self, k: K) -> Option<V> {
        tracing::trace!("Get");
        if self.map.read().await.contains_key(&k) {
            tracing::debug!("Contains key");
            return self.map.read().await.get(&k).cloned();
        }

        let mut notifiers = self.notifiers.lock().await;
        let notify = notifiers
            .entry(k.clone())
            .or_insert(Arc::new(Notify::new()))
            .clone();
        drop(notifiers);

        tracing::trace!("Waiting...");
        notify.notified().await;
        tracing::trace!("Notified, data is available");
        self.map.read().await.get(&k).cloned()
    }

    pub async fn remove(&self, k: K) -> Option<V> {
        self.map.write().await.remove(&k)
    }

    pub async fn len(&self) -> usize {
        self.map.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.map.read().await.is_empty()
    }
}

impl<K, V> Default for UnboundedNapMap<K, V>
where
    K: Eq + Hash + Clone + Debug,
    V: Clone + Debug,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> Debug for UnboundedNapMap<K, V>
where
    K: Eq + Hash + Clone + Debug,
    V: Clone + Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnboundedNapMap")
            .field("map", &self.map)
            .field("notifiers", &self.notifiers)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::UnboundedNapMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tracing_subscriber::EnvFilter;

    // Add this to a test to see the logs
    fn _tracing_sub() {
        let env_filter =
            EnvFilter::from_default_env().add_directive("napmap=trace".parse().unwrap());
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }

    #[tokio::test]
    async fn it_should_wait_until_data_is_inserted() {
        let napmap = Arc::new(UnboundedNapMap::new());

        tokio::spawn({
            let map = napmap.clone();
            async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                map.insert("key", 7).await;
            }
        });

        let res = napmap.get("key").await.unwrap();
        assert_eq!(res, 7);
    }

    #[tokio::test]
    async fn it_should_notify_all_waiters() {
        let napmap = Arc::new(UnboundedNapMap::new());

        tokio::spawn({
            let map = napmap.clone();
            async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                map.insert("key", 7).await;
            }
        });

        let first_handle = tokio::spawn({
            let map = napmap.clone();
            async move {
                let res = map.get("key").await.unwrap();
                assert_eq!(res, 7);
            }
        });

        let second_handle = tokio::spawn({
            let map = napmap.clone();
            async move {
                let res = map.get("key").await.unwrap();
                assert_eq!(res, 7);
            }
        });

        first_handle.await.unwrap();
        second_handle.await.unwrap();
    }
}
