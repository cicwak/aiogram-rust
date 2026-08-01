//! Finite-state-machine context, strategies, key builders, storage, and event isolation.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::RwLock;

use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::dispatcher::{Middleware, Next, UpdateContext};
use crate::error::{Error, Result};
use crate::filters::{Filter, FnFilter};

pub const DEFAULT_DESTINY: &str = "default";
pub type StateData = BTreeMap<String, Value>;

/// A state name with optional group qualification (`group:state`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct State {
    group: Option<Cow<'static, str>>,
    name: Option<Cow<'static, str>>,
    any: bool,
}

impl State {
    pub const fn new(group: &'static str, name: &'static str) -> Self {
        Self {
            group: Some(Cow::Borrowed(group)),
            name: Some(Cow::Borrowed(name)),
            any: false,
        }
    }

    pub fn ungrouped(name: impl Into<String>) -> Self {
        Self {
            group: None,
            name: Some(Cow::Owned(name.into())),
            any: false,
        }
    }

    pub const fn any() -> Self {
        Self {
            group: None,
            name: None,
            any: true,
        }
    }

    pub fn full_name(&self) -> Option<String> {
        if self.any {
            return Some("*".to_owned());
        }
        match (&self.group, &self.name) {
            (Some(group), Some(name)) => Some(format!("{group}:{name}")),
            (None, Some(name)) => Some(name.to_string()),
            _ => None,
        }
    }

    pub fn matches(&self, raw_state: Option<&str>) -> bool {
        self.any || self.full_name().as_deref() == raw_state
    }
}

impl fmt::Display for State {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.full_name().as_deref().unwrap_or_default())
    }
}

impl From<State> for String {
    fn from(value: State) -> Self {
        value.full_name().unwrap_or_default()
    }
}

impl From<&State> for String {
    fn from(value: &State) -> Self {
        value.full_name().unwrap_or_default()
    }
}

/// Runtime description of a state group, including nested groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatesGroup {
    pub name: String,
    pub states: Vec<State>,
    pub children: Vec<StatesGroup>,
}

impl StatesGroup {
    pub fn new(name: impl Into<String>, states: Vec<State>) -> Self {
        Self {
            name: name.into(),
            states,
            children: Vec::new(),
        }
    }

    pub fn child(mut self, child: StatesGroup) -> Self {
        self.children.push(child);
        self
    }

    pub fn contains(&self, raw_state: &str) -> bool {
        self.states
            .iter()
            .any(|state| state.matches(Some(raw_state)))
            || self.children.iter().any(|child| child.contains(raw_state))
    }
}

/// Declares an idiomatic Rust state group while preserving aiogram state names.
#[macro_export]
macro_rules! states_group {
    ($visibility:vis $group:ident { $($constant:ident => $name:literal),+ $(,)? }) => {
        $visibility struct $group;
        impl $group {
            $(
                $visibility const $constant: $crate::fsm::State =
                    $crate::fsm::State::new(stringify!($group), $name);
            )+

            $visibility fn group() -> $crate::fsm::StatesGroup {
                $crate::fsm::StatesGroup::new(
                    stringify!($group),
                    vec![$(Self::$constant.clone()),+],
                )
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageKey {
    pub bot_id: i64,
    pub chat_id: i64,
    pub user_id: i64,
    pub thread_id: Option<i64>,
    pub business_connection_id: Option<String>,
    pub destiny: String,
}

impl StorageKey {
    pub fn new(bot_id: i64, chat_id: i64, user_id: i64) -> Self {
        Self {
            bot_id,
            chat_id,
            user_id,
            thread_id: None,
            business_connection_id: None,
            destiny: DEFAULT_DESTINY.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsmStrategy {
    UserInChat,
    Chat,
    GlobalUser,
    UserInTopic,
    ChatTopic,
}

impl FsmStrategy {
    pub fn apply(
        self,
        chat_id: i64,
        user_id: i64,
        thread_id: Option<i64>,
    ) -> (i64, i64, Option<i64>) {
        match self {
            Self::Chat => (chat_id, chat_id, None),
            Self::GlobalUser => (user_id, user_id, None),
            Self::UserInTopic => (chat_id, user_id, thread_id),
            Self::ChatTopic => (chat_id, chat_id, thread_id),
            Self::UserInChat => (chat_id, user_id, None),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPart {
    Data,
    State,
    Lock,
}

impl KeyPart {
    fn as_str(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::State => "state",
            Self::Lock => "lock",
        }
    }
}

pub trait KeyBuilder: Send + Sync {
    fn build(&self, key: &StorageKey, part: Option<KeyPart>) -> Result<String>;
}

/// Produces keys compatible with aiogram's `DefaultKeyBuilder` format.
#[derive(Debug, Clone)]
pub struct DefaultKeyBuilder {
    pub prefix: String,
    pub separator: String,
    pub with_bot_id: bool,
    pub with_business_connection_id: bool,
    pub with_destiny: bool,
}

impl Default for DefaultKeyBuilder {
    fn default() -> Self {
        Self {
            prefix: "fsm".to_owned(),
            separator: ":".to_owned(),
            with_bot_id: false,
            with_business_connection_id: false,
            with_destiny: false,
        }
    }
}

impl DefaultKeyBuilder {
    pub fn with_bot_id(mut self, value: bool) -> Self {
        self.with_bot_id = value;
        self
    }

    pub fn with_business_connection_id(mut self, value: bool) -> Self {
        self.with_business_connection_id = value;
        self
    }

    pub fn with_destiny(mut self, value: bool) -> Self {
        self.with_destiny = value;
        self
    }
}

impl KeyBuilder for DefaultKeyBuilder {
    fn build(&self, key: &StorageKey, part: Option<KeyPart>) -> Result<String> {
        let mut parts = vec![self.prefix.clone()];
        if self.with_bot_id {
            parts.push(key.bot_id.to_string());
        }
        if self.with_business_connection_id
            && let Some(connection_id) = &key.business_connection_id
        {
            parts.push(connection_id.clone());
        }
        parts.push(key.chat_id.to_string());
        if let Some(thread_id) = key.thread_id {
            parts.push(thread_id.to_string());
        }
        parts.push(key.user_id.to_string());
        if self.with_destiny {
            parts.push(key.destiny.clone());
        } else if key.destiny != DEFAULT_DESTINY {
            return Err(Error::Fsm(
                "key builder must enable with_destiny for a non-default destiny".to_owned(),
            ));
        }
        if let Some(part) = part {
            parts.push(part.as_str().to_owned());
        }
        Ok(parts.join(&self.separator))
    }
}

#[async_trait]
pub trait Storage: Send + Sync {
    async fn set_state(&self, key: &StorageKey, state: Option<String>) -> Result<()>;
    async fn get_state(&self, key: &StorageKey) -> Result<Option<String>>;
    async fn set_data(&self, key: &StorageKey, data: StateData) -> Result<()>;
    async fn get_data(&self, key: &StorageKey) -> Result<StateData>;

    async fn get_value(&self, key: &StorageKey, name: &str) -> Result<Option<Value>> {
        Ok(self.get_data(key).await?.get(name).cloned())
    }

    async fn update_data(&self, key: &StorageKey, values: StateData) -> Result<StateData> {
        let mut data = self.get_data(key).await?;
        data.extend(values);
        self.set_data(key, data.clone()).await?;
        Ok(data)
    }

    async fn clear(&self, key: &StorageKey) -> Result<()> {
        self.set_state(key, None).await?;
        self.set_data(key, StateData::new()).await
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
struct MemoryRecord {
    state: Option<String>,
    data: StateData,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    records: Arc<DashMap<StorageKey, MemoryRecord>>,
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn set_state(&self, key: &StorageKey, state: Option<String>) -> Result<()> {
        self.records.entry(key.clone()).or_default().state = state;
        Ok(())
    }

    async fn get_state(&self, key: &StorageKey) -> Result<Option<String>> {
        Ok(self
            .records
            .get(key)
            .and_then(|record| record.state.clone()))
    }

    async fn set_data(&self, key: &StorageKey, data: StateData) -> Result<()> {
        self.records.entry(key.clone()).or_default().data = data;
        Ok(())
    }

    async fn get_data(&self, key: &StorageKey) -> Result<StateData> {
        Ok(self
            .records
            .get(key)
            .map(|record| record.data.clone())
            .unwrap_or_default())
    }

    async fn update_data(&self, key: &StorageKey, values: StateData) -> Result<StateData> {
        let mut record = self.records.entry(key.clone()).or_default();
        record.data.extend(values);
        Ok(record.data.clone())
    }

    async fn clear(&self, key: &StorageKey) -> Result<()> {
        self.records.remove(key);
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(feature = "fsm-redis")]
#[derive(Clone)]
pub struct RedisStorage {
    connection: redis::aio::ConnectionManager,
    key_builder: Arc<dyn KeyBuilder>,
    state_ttl: Option<std::time::Duration>,
    data_ttl: Option<std::time::Duration>,
}

#[cfg(feature = "fsm-redis")]
impl RedisStorage {
    pub async fn from_url(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)?;
        let connection = client.get_connection_manager().await?;
        Ok(Self {
            connection,
            key_builder: Arc::new(DefaultKeyBuilder::default()),
            state_ttl: None,
            data_ttl: None,
        })
    }

    pub fn key_builder(mut self, value: impl KeyBuilder + 'static) -> Self {
        self.key_builder = Arc::new(value);
        self
    }

    pub fn state_ttl(mut self, value: std::time::Duration) -> Self {
        self.state_ttl = Some(value);
        self
    }

    pub fn data_ttl(mut self, value: std::time::Duration) -> Self {
        self.data_ttl = Some(value);
        self
    }

    pub fn create_isolation(&self) -> RedisEventIsolation {
        RedisEventIsolation {
            connection: self.connection.clone(),
            key_builder: self.key_builder.clone(),
            lock_timeout: std::time::Duration::from_secs(60),
            retry_delay: std::time::Duration::from_millis(100),
        }
    }

    async fn set_value(
        &self,
        key: String,
        value: Option<String>,
        ttl: Option<std::time::Duration>,
    ) -> Result<()> {
        let mut connection = self.connection.clone();
        match value {
            Some(value) => {
                let mut command = redis::cmd("SET");
                command.arg(key).arg(value);
                if let Some(ttl) = ttl {
                    command
                        .arg("PX")
                        .arg(ttl.as_millis().min(u64::MAX as u128) as u64);
                }
                command.query_async::<()>(&mut connection).await?;
            }
            None => {
                redis::cmd("DEL")
                    .arg(key)
                    .query_async::<()>(&mut connection)
                    .await?;
            }
        }
        Ok(())
    }

    async fn get_value_raw(&self, key: String) -> Result<Option<String>> {
        let mut connection = self.connection.clone();
        Ok(redis::cmd("GET")
            .arg(key)
            .query_async::<Option<String>>(&mut connection)
            .await?)
    }
}

#[cfg(feature = "fsm-redis")]
#[async_trait]
impl Storage for RedisStorage {
    async fn set_state(&self, key: &StorageKey, state: Option<String>) -> Result<()> {
        self.set_value(
            self.key_builder.build(key, Some(KeyPart::State))?,
            state,
            self.state_ttl,
        )
        .await
    }

    async fn get_state(&self, key: &StorageKey) -> Result<Option<String>> {
        self.get_value_raw(self.key_builder.build(key, Some(KeyPart::State))?)
            .await
    }

    async fn set_data(&self, key: &StorageKey, data: StateData) -> Result<()> {
        let value = if data.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&data)?)
        };
        self.set_value(
            self.key_builder.build(key, Some(KeyPart::Data))?,
            value,
            self.data_ttl,
        )
        .await
    }

    async fn get_data(&self, key: &StorageKey) -> Result<StateData> {
        match self
            .get_value_raw(self.key_builder.build(key, Some(KeyPart::Data))?)
            .await?
        {
            Some(value) => Ok(serde_json::from_str(&value)?),
            None => Ok(StateData::new()),
        }
    }
}

/// Distributed event isolation compatible with aiogram's
/// `RedisEventIsolation`. Locks use a finite lease and are released by an
/// atomic token comparison so one worker cannot delete another worker's lock.
#[cfg(feature = "fsm-redis")]
#[derive(Clone)]
pub struct RedisEventIsolation {
    connection: redis::aio::ConnectionManager,
    key_builder: Arc<dyn KeyBuilder>,
    lock_timeout: std::time::Duration,
    retry_delay: std::time::Duration,
}

#[cfg(feature = "fsm-redis")]
impl RedisEventIsolation {
    pub async fn from_url(url: &str) -> Result<Self> {
        Ok(RedisStorage::from_url(url).await?.create_isolation())
    }

    pub fn key_builder(mut self, value: impl KeyBuilder + 'static) -> Self {
        self.key_builder = Arc::new(value);
        self
    }

    pub fn lock_timeout(mut self, value: std::time::Duration) -> Self {
        self.lock_timeout = value;
        self
    }

    pub fn retry_delay(mut self, value: std::time::Duration) -> Self {
        self.retry_delay = value;
        self
    }

    fn token() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
        let sequence = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{}-{timestamp}-{sequence}", std::process::id())
    }
}

#[cfg(feature = "fsm-redis")]
pub struct RedisLockGuard {
    connection: redis::aio::ConnectionManager,
    key: String,
    token: String,
}

#[cfg(feature = "fsm-redis")]
impl Drop for RedisLockGuard {
    fn drop(&mut self) {
        let mut connection = self.connection.clone();
        let key = self.key.clone();
        let token = self.token.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            let script = redis::Script::new(
                "if redis.call('get', KEYS[1]) == ARGV[1] then return redis.call('del', KEYS[1]) else return 0 end",
            );
            if let Err(error) = script
                .key(key)
                .arg(token)
                .invoke_async::<i64>(&mut connection)
                .await
            {
                tracing::warn!(%error, "failed to release Redis FSM isolation lock");
            }
        });
    }
}

/// MongoDB FSM storage compatible with aiogram's `PyMongoStorage` document
/// layout. State and data share one document keyed by `KeyBuilder::build(key,
/// None)`.
#[cfg(feature = "fsm-mongodb")]
#[derive(Clone)]
pub struct MongoStorage {
    client: mongodb::Client,
    collection: mongodb::Collection<mongodb::bson::Document>,
    key_builder: Arc<dyn KeyBuilder>,
}

#[cfg(feature = "fsm-mongodb")]
impl MongoStorage {
    pub fn new(
        client: mongodb::Client,
        database_name: impl AsRef<str>,
        collection_name: impl AsRef<str>,
    ) -> Self {
        let collection = client
            .database(database_name.as_ref())
            .collection(collection_name.as_ref());
        Self {
            client,
            collection,
            key_builder: Arc::new(DefaultKeyBuilder::default()),
        }
    }

    pub async fn from_url(url: &str) -> Result<Self> {
        let client = mongodb::Client::with_uri_str(url).await?;
        Ok(Self::new(client, "aiogram_fsm", "states_and_data"))
    }

    pub fn key_builder(mut self, value: impl KeyBuilder + 'static) -> Self {
        self.key_builder = Arc::new(value);
        self
    }

    pub fn namespace(
        mut self,
        database_name: impl AsRef<str>,
        collection_name: impl AsRef<str>,
    ) -> Self {
        self.collection = self
            .client
            .database(database_name.as_ref())
            .collection(collection_name.as_ref());
        self
    }

    fn document_id(&self, key: &StorageKey) -> Result<String> {
        self.key_builder.build(key, None)
    }

    async fn unset_and_remove_empty(&self, id: &str, field: &str) -> Result<()> {
        use mongodb::bson::{Bson, doc};
        use mongodb::options::ReturnDocument;

        let updated = self
            .collection
            .find_one_and_update(
                doc! { "_id": id },
                doc! { "$unset": { field: Bson::Int32(1) } },
            )
            .return_document(ReturnDocument::After)
            .projection(doc! { "_id": 0 })
            .await?;
        if updated
            .as_ref()
            .is_some_and(mongodb::bson::Document::is_empty)
        {
            self.collection.delete_one(doc! { "_id": id }).await?;
        }
        Ok(())
    }

    fn decode_data(document: Option<mongodb::bson::Document>) -> Result<StateData> {
        let Some(data) = document.and_then(|mut document| document.remove("data")) else {
            return Ok(StateData::new());
        };
        mongodb::bson::from_bson(data)
            .map_err(|error| Error::Fsm(format!("invalid MongoDB FSM data: {error}")))
    }
}

#[cfg(feature = "fsm-mongodb")]
#[async_trait]
impl Storage for MongoStorage {
    async fn set_state(&self, key: &StorageKey, state: Option<String>) -> Result<()> {
        use mongodb::bson::doc;

        let id = self.document_id(key)?;
        match state {
            Some(state) => {
                self.collection
                    .update_one(doc! { "_id": &id }, doc! { "$set": { "state": state } })
                    .upsert(true)
                    .await?;
                Ok(())
            }
            None => self.unset_and_remove_empty(&id, "state").await,
        }
    }

    async fn get_state(&self, key: &StorageKey) -> Result<Option<String>> {
        use mongodb::bson::doc;

        let document = self
            .collection
            .find_one(doc! { "_id": self.document_id(key)? })
            .projection(doc! { "_id": 0, "state": 1 })
            .await?;
        Ok(document.and_then(|document| document.get_str("state").ok().map(str::to_owned)))
    }

    async fn set_data(&self, key: &StorageKey, data: StateData) -> Result<()> {
        use mongodb::bson::{doc, to_bson};

        let id = self.document_id(key)?;
        if data.is_empty() {
            return self.unset_and_remove_empty(&id, "data").await;
        }
        self.collection
            .update_one(
                doc! { "_id": id },
                doc! { "$set": { "data": to_bson(&data).map_err(|error| {
                    Error::Fsm(format!("cannot encode MongoDB FSM data: {error}"))
                })? } },
            )
            .upsert(true)
            .await?;
        Ok(())
    }

    async fn get_data(&self, key: &StorageKey) -> Result<StateData> {
        use mongodb::bson::doc;

        let document = self
            .collection
            .find_one(doc! { "_id": self.document_id(key)? })
            .projection(doc! { "_id": 0, "data": 1 })
            .await?;
        Self::decode_data(document)
    }

    async fn update_data(&self, key: &StorageKey, values: StateData) -> Result<StateData> {
        use mongodb::bson::{Document, doc, to_bson};
        use mongodb::options::ReturnDocument;

        if values.is_empty() {
            return self.get_data(key).await;
        }
        let mut fields = Document::new();
        for (name, value) in values {
            fields.insert(
                format!("data.{name}"),
                to_bson(&value).map_err(|error| {
                    Error::Fsm(format!("cannot encode MongoDB FSM value: {error}"))
                })?,
            );
        }
        let document = self
            .collection
            .find_one_and_update(
                doc! { "_id": self.document_id(key)? },
                doc! { "$set": fields },
            )
            .upsert(true)
            .return_document(ReturnDocument::After)
            .projection(doc! { "_id": 0, "data": 1 })
            .await?;
        Self::decode_data(document)
    }

    async fn close(&self) -> Result<()> {
        self.client.clone().shutdown().await;
        Ok(())
    }
}

#[derive(Clone)]
pub struct FsmContext {
    storage: Arc<dyn Storage>,
    key: StorageKey,
}

impl FsmContext {
    pub fn new(storage: Arc<dyn Storage>, key: StorageKey) -> Self {
        Self { storage, key }
    }

    pub fn key(&self) -> &StorageKey {
        &self.key
    }
    pub fn with_destiny(&self, destiny: impl Into<String>) -> Self {
        let mut key = self.key.clone();
        key.destiny = destiny.into();
        Self {
            storage: self.storage.clone(),
            key,
        }
    }
    pub async fn set_state(&self, state: impl Into<String>) -> Result<()> {
        self.storage.set_state(&self.key, Some(state.into())).await
    }
    pub async fn set_raw_state(&self, state: Option<String>) -> Result<()> {
        self.storage.set_state(&self.key, state).await
    }
    pub async fn get_state(&self) -> Result<Option<String>> {
        self.storage.get_state(&self.key).await
    }
    pub async fn set_data(&self, data: StateData) -> Result<()> {
        self.storage.set_data(&self.key, data).await
    }
    pub async fn get_data(&self) -> Result<StateData> {
        self.storage.get_data(&self.key).await
    }
    pub async fn get_value(&self, name: &str) -> Result<Option<Value>> {
        self.storage.get_value(&self.key, name).await
    }
    pub async fn update_data(&self, values: StateData) -> Result<StateData> {
        self.storage.update_data(&self.key, values).await
    }
    pub async fn clear(&self) -> Result<()> {
        self.storage.clear(&self.key).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateRecord {
    pub state: Option<String>,
    pub data: StateData,
}

/// Scene/FSM history stored in a separate destiny, matching aiogram's rollback model.
#[derive(Clone)]
pub struct HistoryManager {
    state: FsmContext,
    history: FsmContext,
    size: usize,
}

impl HistoryManager {
    pub fn new(state: FsmContext) -> Self {
        Self::with_options(state, "scenes_history", 10)
    }

    pub fn with_options(state: FsmContext, destiny: impl Into<String>, size: usize) -> Self {
        let history = state.with_destiny(destiny);
        Self {
            state,
            history,
            size,
        }
    }

    pub async fn push(&self, state: Option<String>, data: StateData) -> Result<()> {
        let mut history = self.all().await?;
        history.push(StateRecord { state, data });
        if history.len() > self.size {
            let remove = history.len() - self.size;
            history.drain(..remove);
        }
        self.store(history).await
    }

    pub async fn pop(&self) -> Result<Option<StateRecord>> {
        let mut history = self.all().await?;
        let record = history.pop();
        self.store(history).await?;
        Ok(record)
    }

    pub async fn get(&self) -> Result<Option<StateRecord>> {
        Ok(self.all().await?.pop())
    }

    pub async fn all(&self) -> Result<Vec<StateRecord>> {
        let value = self.history.get_value("history").await?;
        match value {
            Some(value) => Ok(serde_json::from_value(value)?),
            None => Ok(Vec::new()),
        }
    }

    pub async fn clear(&self) -> Result<()> {
        self.history.set_data(StateData::new()).await
    }

    pub async fn snapshot(&self) -> Result<()> {
        self.push(self.state.get_state().await?, self.state.get_data().await?)
            .await
    }

    pub async fn rollback(&self) -> Result<Option<String>> {
        let record = self.pop().await?;
        match record {
            Some(record) => {
                self.state.set_raw_state(record.state.clone()).await?;
                self.state.set_data(record.data).await?;
                Ok(record.state)
            }
            None => {
                self.state.clear().await?;
                Ok(None)
            }
        }
    }

    async fn store(&self, history: Vec<StateRecord>) -> Result<()> {
        if history.is_empty() {
            self.history.set_data(StateData::new()).await
        } else {
            self.history
                .set_data(BTreeMap::from([(
                    "history".to_owned(),
                    serde_json::to_value(history)?,
                )]))
                .await
        }
    }
}

/// Minimal scene navigation primitive with snapshots and rollback.
#[derive(Clone)]
pub struct SceneWizard {
    state: FsmContext,
    history: HistoryManager,
}

impl SceneWizard {
    pub fn new(state: FsmContext) -> Self {
        Self {
            history: HistoryManager::new(state.clone()),
            state,
        }
    }

    pub async fn goto(&self, state: impl Into<String>) -> Result<()> {
        self.history.snapshot().await?;
        self.state.set_state(state).await
    }

    pub async fn goto_with_data(&self, state: impl Into<String>, data: StateData) -> Result<()> {
        self.history.snapshot().await?;
        self.state.set_state(state).await?;
        self.state.set_data(data).await
    }

    pub async fn leave(&self) -> Result<()> {
        self.state.set_raw_state(None).await
    }

    pub async fn exit(&self) -> Result<()> {
        self.state.set_raw_state(None).await?;
        self.history.clear().await
    }

    pub async fn back(&self) -> Result<Option<String>> {
        self.history.rollback().await
    }
}

pub type SceneFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;
type SceneCallback =
    Arc<dyn Fn(UpdateContext, ScenesManager) -> SceneFuture + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SceneAction {
    Enter,
    Leave,
    Exit,
    Back,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum After {
    Exit,
    Back,
    Goto(String),
}

impl After {
    pub fn goto(scene: impl Into<String>) -> Self {
        Self::Goto(scene.into())
    }
}

#[derive(Clone)]
struct SceneRoute {
    event_type: String,
    filter: Arc<dyn Filter>,
    handler: SceneCallback,
    after: Option<After>,
}

#[derive(Clone)]
pub struct SceneDefinition {
    state: String,
    routes: Vec<SceneRoute>,
    actions: BTreeMap<(SceneAction, String), SceneCallback>,
    reset_data_on_enter: bool,
    reset_history_on_enter: bool,
    callback_query_without_state: bool,
}

impl SceneDefinition {
    pub fn state(&self) -> &str {
        &self.state
    }
}

pub struct SceneBuilder {
    definition: SceneDefinition,
}

impl SceneBuilder {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            definition: SceneDefinition {
                state: state.into(),
                routes: Vec::new(),
                actions: BTreeMap::new(),
                reset_data_on_enter: false,
                reset_history_on_enter: false,
                callback_query_without_state: false,
            },
        }
    }

    pub fn reset_data_on_enter(mut self, value: bool) -> Self {
        self.definition.reset_data_on_enter = value;
        self
    }

    pub fn reset_history_on_enter(mut self, value: bool) -> Self {
        self.definition.reset_history_on_enter = value;
        self
    }

    pub fn callback_query_without_state(mut self, value: bool) -> Self {
        self.definition.callback_query_without_state = value;
        self
    }

    pub fn on<F, Fut>(
        mut self,
        event_type: impl Into<String>,
        filter: impl Filter + 'static,
        handler: F,
    ) -> Self
    where
        F: Fn(UpdateContext, ScenesManager) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.definition.routes.push(SceneRoute {
            event_type: event_type.into(),
            filter: Arc::new(filter),
            handler: Arc::new(move |context, scenes| Box::pin(handler(context, scenes))),
            after: None,
        });
        self
    }

    pub fn on_after<F, Fut>(
        mut self,
        event_type: impl Into<String>,
        filter: impl Filter + 'static,
        after: After,
        handler: F,
    ) -> Self
    where
        F: Fn(UpdateContext, ScenesManager) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.definition.routes.push(SceneRoute {
            event_type: event_type.into(),
            filter: Arc::new(filter),
            handler: Arc::new(move |context, scenes| Box::pin(handler(context, scenes))),
            after: Some(after),
        });
        self
    }

    pub fn message<F, Fut>(self, filter: impl Filter + 'static, handler: F) -> Self
    where
        F: Fn(UpdateContext, ScenesManager) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on("message", filter, handler)
    }

    pub fn message_after<F, Fut>(
        self,
        filter: impl Filter + 'static,
        after: After,
        handler: F,
    ) -> Self
    where
        F: Fn(UpdateContext, ScenesManager) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on_after("message", filter, after, handler)
    }

    pub fn callback_query<F, Fut>(self, filter: impl Filter + 'static, handler: F) -> Self
    where
        F: Fn(UpdateContext, ScenesManager) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on("callback_query", filter, handler)
    }

    pub fn action<F, Fut>(
        mut self,
        action: SceneAction,
        event_type: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(UpdateContext, ScenesManager) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.definition.actions.insert(
            (action, event_type.into()),
            Arc::new(move |context, scenes| Box::pin(handler(context, scenes))),
        );
        self
    }

    pub fn build(self) -> SceneDefinition {
        self.definition
    }
}

#[derive(Clone, Default)]
pub struct SceneRegistry {
    scenes: Arc<RwLock<BTreeMap<String, SceneDefinition>>>,
}

impl SceneRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, scene: SceneDefinition) -> Result<()> {
        let mut scenes = self
            .scenes
            .write()
            .map_err(|_| Error::Fsm("scene registry lock poisoned".to_owned()))?;
        if scenes.contains_key(&scene.state) {
            return Err(Error::Fsm(format!(
                "scene with state {:?} is already registered",
                scene.state
            )));
        }
        scenes.insert(scene.state.clone(), scene);
        Ok(())
    }

    pub fn contains(&self, state: &str) -> bool {
        self.scenes
            .read()
            .is_ok_and(|scenes| scenes.contains_key(state))
    }

    fn get(&self, state: &str) -> Result<SceneDefinition> {
        self.scenes
            .read()
            .map_err(|_| Error::Fsm("scene registry lock poisoned".to_owned()))?
            .get(state)
            .cloned()
            .ok_or_else(|| Error::Fsm(format!("scene {state:?} is not registered")))
    }

    pub fn manager(&self, context: &UpdateContext) -> Result<ScenesManager> {
        let state = context
            .dependency::<FsmContext>()
            .ok_or_else(|| Error::Fsm("scene handling requires FsmMiddleware".to_owned()))?;
        Ok(ScenesManager::new(
            self.clone(),
            context.clone(),
            state.as_ref().clone(),
        ))
    }

    /// Builds a router containing every state-gated scene observer.
    pub fn router(&self) -> Result<crate::Router> {
        let definitions: Vec<_> = self
            .scenes
            .read()
            .map_err(|_| Error::Fsm("scene registry lock poisoned".to_owned()))?
            .values()
            .cloned()
            .collect();
        let mut router = crate::Router::named("scenes");
        for definition in definitions {
            for route in definition.routes.clone() {
                let state = definition.state.clone();
                let filter = route.filter.clone();
                let allow_without_state =
                    definition.callback_query_without_state && route.event_type == "callback_query";
                let state_filter = FnFilter::new(move |context| {
                    let state = state.clone();
                    let filter = filter.clone();
                    Box::pin(async move {
                        let Some(fsm) = context.dependency::<FsmContext>() else {
                            return false;
                        };
                        if !allow_without_state
                            && fsm.get_state().await.ok().flatten().as_deref()
                                != Some(state.as_str())
                        {
                            return false;
                        }
                        filter.check(context).await
                    })
                });
                let registry = self.clone();
                let handler = route.handler.clone();
                let after = route.after.clone();
                router.event(route.event_type, state_filter, move |context| {
                    let registry = registry.clone();
                    let handler = handler.clone();
                    let after = after.clone();
                    async move {
                        let scenes = registry.manager(&context)?;
                        let context = context.with_dependency(scenes.clone());
                        handler(context, scenes.clone()).await?;
                        if let Some(after) = after {
                            scenes.execute_after(after).await?;
                        }
                        Ok(())
                    }
                });
            }
        }
        Ok(router)
    }
}

#[derive(Clone)]
pub struct ScenesManager {
    registry: SceneRegistry,
    context: UpdateContext,
    state: FsmContext,
    history: HistoryManager,
}

impl ScenesManager {
    fn new(registry: SceneRegistry, context: UpdateContext, state: FsmContext) -> Self {
        Self {
            registry,
            context,
            history: HistoryManager::new(state.clone()),
            state,
        }
    }

    pub async fn active_state(&self) -> Result<Option<String>> {
        self.state.get_state().await
    }

    async fn run_action(&self, definition: &SceneDefinition, action: SceneAction) -> Result<bool> {
        let Some(event_type) = self.context.event_type() else {
            return Ok(false);
        };
        let Some(handler) = definition
            .actions
            .get(&(action, event_type.to_owned()))
            .cloned()
        else {
            return Ok(false);
        };
        let context = self.context.clone().with_dependency(self.clone());
        handler(context, self.clone()).await?;
        Ok(true)
    }

    async fn enter_unchecked(&self, scene: Option<&str>) -> Result<()> {
        let Some(scene) = scene else {
            return self.state.set_raw_state(None).await;
        };
        let definition = self.registry.get(scene)?;
        if definition.reset_data_on_enter {
            self.state.set_data(StateData::new()).await?;
        }
        if definition.reset_history_on_enter {
            self.history.clear().await?;
        }
        self.state.set_state(definition.state.clone()).await?;
        self.run_action(&definition, SceneAction::Enter).await?;
        Ok(())
    }

    pub async fn enter(&self, scene: Option<&str>) -> Result<()> {
        if self
            .state
            .get_state()
            .await?
            .as_deref()
            .is_some_and(|state| self.registry.contains(state))
        {
            self.exit().await?;
        }
        self.enter_unchecked(scene).await
    }

    pub async fn leave(&self) -> Result<()> {
        let Some(active) = self.state.get_state().await? else {
            return Ok(());
        };
        let definition = self.registry.get(&active)?;
        self.history.snapshot().await?;
        self.run_action(&definition, SceneAction::Leave).await?;
        Ok(())
    }

    pub async fn goto(&self, scene: &str) -> Result<()> {
        self.leave().await?;
        self.enter_unchecked(Some(scene)).await
    }

    pub async fn exit(&self) -> Result<()> {
        if let Some(active) = self.state.get_state().await?
            && let Ok(definition) = self.registry.get(&active)
        {
            self.run_action(&definition, SceneAction::Exit).await?;
        }
        self.state.set_raw_state(None).await?;
        self.history.clear().await
    }

    pub async fn back(&self) -> Result<Option<String>> {
        if let Some(active) = self.state.get_state().await?
            && let Ok(definition) = self.registry.get(&active)
        {
            self.run_action(&definition, SceneAction::Leave).await?;
        }
        let restored = self.history.rollback().await?;
        if let Some(state) = &restored
            && let Ok(definition) = self.registry.get(state)
        {
            self.run_action(&definition, SceneAction::Enter).await?;
        }
        Ok(restored)
    }

    pub async fn retake(&self) -> Result<()> {
        let active = self
            .active_state()
            .await?
            .ok_or_else(|| Error::Fsm("cannot retake without an active scene".to_owned()))?;
        self.goto(&active).await
    }

    pub async fn execute_after(&self, after: After) -> Result<()> {
        match after {
            After::Exit => self.exit().await,
            After::Back => self.back().await.map(|_| ()),
            After::Goto(scene) => self.goto(&scene).await,
        }
    }

    pub async fn set_data(&self, data: StateData) -> Result<()> {
        self.state.set_data(data).await
    }

    pub async fn get_data(&self) -> Result<StateData> {
        self.state.get_data().await
    }

    pub async fn get_value(&self, key: &str) -> Result<Option<Value>> {
        self.state.get_value(key).await
    }

    pub async fn update_data(&self, data: StateData) -> Result<StateData> {
        self.state.update_data(data).await
    }

    pub async fn clear_data(&self) -> Result<()> {
        self.state.set_data(StateData::new()).await
    }
}

pub enum IsolationGuard {
    Disabled,
    Locked(OwnedMutexGuard<()>),
    #[cfg(feature = "fsm-redis")]
    Redis(RedisLockGuard),
}

#[async_trait]
pub trait EventIsolation: Send + Sync {
    async fn lock(&self, key: &StorageKey) -> Result<IsolationGuard>;
    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct DisabledEventIsolation;

#[async_trait]
impl EventIsolation for DisabledEventIsolation {
    async fn lock(&self, _key: &StorageKey) -> Result<IsolationGuard> {
        Ok(IsolationGuard::Disabled)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SimpleEventIsolation {
    locks: Arc<DashMap<StorageKey, Arc<Mutex<()>>>>,
}

/// Dispatcher middleware that derives an FSM key and injects `FsmContext`.
#[derive(Clone)]
pub struct FsmMiddleware {
    storage: Arc<dyn Storage>,
    strategy: FsmStrategy,
    isolation: Arc<dyn EventIsolation>,
}

impl FsmMiddleware {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            strategy: FsmStrategy::UserInChat,
            isolation: Arc::new(DisabledEventIsolation),
        }
    }

    pub fn strategy(mut self, value: FsmStrategy) -> Self {
        self.strategy = value;
        self
    }

    pub fn event_isolation(mut self, value: impl EventIsolation + 'static) -> Self {
        self.isolation = Arc::new(value);
        self
    }

    pub fn storage(&self) -> Arc<dyn Storage> {
        self.storage.clone()
    }

    pub fn get_context(
        &self,
        bot_id: i64,
        chat_id: i64,
        user_id: i64,
        thread_id: Option<i64>,
        business_connection_id: Option<String>,
        destiny: impl Into<String>,
    ) -> FsmContext {
        FsmContext::new(
            self.storage.clone(),
            StorageKey {
                bot_id,
                chat_id,
                user_id,
                thread_id,
                business_connection_id,
                destiny: destiny.into(),
            },
        )
    }

    pub fn resolve_context(
        &self,
        bot_id: i64,
        mut chat_id: Option<i64>,
        mut user_id: Option<i64>,
        thread_id: Option<i64>,
        business_connection_id: Option<String>,
        destiny: impl Into<String>,
    ) -> Option<FsmContext> {
        if chat_id.is_none() {
            chat_id = user_id;
        } else if user_id.is_none()
            && matches!(self.strategy, FsmStrategy::Chat | FsmStrategy::ChatTopic)
        {
            user_id = chat_id;
        }
        let (chat_id, user_id) = (chat_id?, user_id?);
        let (chat_id, user_id, thread_id) = self.strategy.apply(chat_id, user_id, thread_id);
        Some(self.get_context(
            bot_id,
            chat_id,
            user_id,
            thread_id,
            business_connection_id,
            destiny,
        ))
    }

    pub async fn close(&self) -> Result<()> {
        let storage_result = self.storage.close().await;
        let isolation_result = self.isolation.close().await;
        storage_result.and(isolation_result)
    }

    fn key(&self, context: &UpdateContext) -> Option<StorageKey> {
        let event = context.event_context()?;
        let mut chat_id = event.chat_id();
        let mut user_id = event.user_id();
        if chat_id.is_none() {
            chat_id = user_id;
        } else if user_id.is_none()
            && matches!(self.strategy, FsmStrategy::Chat | FsmStrategy::ChatTopic)
        {
            user_id = chat_id;
        }
        let (chat_id, user_id) = (chat_id?, user_id?);
        let thread_id = event.thread_id;
        let (chat_id, user_id, thread_id) = self.strategy.apply(chat_id, user_id, thread_id);
        Some(StorageKey {
            bot_id: context.bot.id(),
            chat_id,
            user_id,
            thread_id,
            business_connection_id: event.business_connection_id.clone(),
            destiny: DEFAULT_DESTINY.to_owned(),
        })
    }

    pub(crate) async fn resolve(
        &self,
        context: &UpdateContext,
    ) -> Result<Option<(FsmContext, IsolationGuard)>> {
        let Some(key) = self.key(context) else {
            return Ok(None);
        };
        let guard = self.isolation.lock(&key).await?;
        Ok(Some((FsmContext::new(self.storage.clone(), key), guard)))
    }
}

#[async_trait]
impl Middleware for FsmMiddleware {
    async fn handle(&self, context: UpdateContext, next: Next) -> Result<()> {
        let Some(key) = self.key(&context) else {
            return next.run(context).await;
        };
        let _guard = self.isolation.lock(&key).await?;
        let fsm = FsmContext::new(self.storage.clone(), key);
        next.run(context.with_dependency(fsm)).await
    }
}

#[async_trait]
impl EventIsolation for SimpleEventIsolation {
    async fn lock(&self, key: &StorageKey) -> Result<IsolationGuard> {
        let lock = self
            .locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        Ok(IsolationGuard::Locked(lock.lock_owned().await))
    }

    async fn close(&self) -> Result<()> {
        self.locks.clear();
        Ok(())
    }
}

#[cfg(feature = "fsm-redis")]
#[async_trait]
impl EventIsolation for RedisEventIsolation {
    async fn lock(&self, key: &StorageKey) -> Result<IsolationGuard> {
        let redis_key = self.key_builder.build(key, Some(KeyPart::Lock))?;
        let token = Self::token();
        let ttl_millis = self.lock_timeout.as_millis().clamp(1, u64::MAX as u128) as u64;
        loop {
            let mut connection = self.connection.clone();
            let acquired = redis::cmd("SET")
                .arg(&redis_key)
                .arg(&token)
                .arg("NX")
                .arg("PX")
                .arg(ttl_millis)
                .query_async::<Option<String>>(&mut connection)
                .await?;
            if acquired.is_some() {
                return Ok(IsolationGuard::Redis(RedisLockGuard {
                    connection: self.connection.clone(),
                    key: redis_key,
                    token,
                }));
            }
            tokio::time::sleep(self.retry_delay).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::states_group! {
        Form {
            NAME => "name",
            AGE => "age",
        }
    }

    #[tokio::test]
    async fn memory_storage_roundtrip_update_and_clear() {
        let context = FsmContext::new(Arc::new(MemoryStorage::default()), StorageKey::new(1, 2, 3));
        context.set_state("form:name").await.unwrap();
        context
            .update_data(BTreeMap::from([("name".to_owned(), Value::from("Ada"))]))
            .await
            .unwrap();
        assert_eq!(
            context.get_state().await.unwrap().as_deref(),
            Some("form:name")
        );
        assert_eq!(
            context.get_value("name").await.unwrap(),
            Some(Value::from("Ada"))
        );
        context.clear().await.unwrap();
        assert_eq!(context.get_state().await.unwrap(), None);
        assert!(context.get_data().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn middleware_exposes_manual_context_and_closes_storage_and_isolation() {
        let storage = Arc::new(MemoryStorage::default());
        let isolation = SimpleEventIsolation::default();
        let middleware = FsmMiddleware::new(storage.clone())
            .strategy(FsmStrategy::Chat)
            .event_isolation(isolation.clone());
        let context = middleware
            .resolve_context(
                123,
                Some(42),
                None,
                Some(7),
                Some("business".to_owned()),
                "manual",
            )
            .unwrap();
        assert_eq!(context.key().bot_id, 123);
        assert_eq!(context.key().chat_id, 42);
        assert_eq!(context.key().user_id, 42);
        assert_eq!(context.key().thread_id, None);
        assert_eq!(
            context.key().business_connection_id.as_deref(),
            Some("business")
        );
        assert_eq!(context.key().destiny, "manual");
        context.set_state("Form:name").await.unwrap();

        let guard = isolation.lock(context.key()).await.unwrap();
        drop(guard);
        assert_eq!(isolation.locks.len(), 1);
        middleware.close().await.unwrap();
        assert!(isolation.locks.is_empty());
        assert_eq!(
            storage.get_state(context.key()).await.unwrap().as_deref(),
            Some("Form:name"),
            "MemoryStorage.close is intentionally a no-op like upstream"
        );
    }

    #[test]
    fn strategies_match_aiogram_key_coordinates() {
        assert_eq!(
            FsmStrategy::UserInChat.apply(10, 20, Some(30)),
            (10, 20, None)
        );
        assert_eq!(FsmStrategy::Chat.apply(10, 20, Some(30)), (10, 10, None));
        assert_eq!(
            FsmStrategy::GlobalUser.apply(10, 20, Some(30)),
            (20, 20, None)
        );
        assert_eq!(
            FsmStrategy::UserInTopic.apply(10, 20, Some(30)),
            (10, 20, Some(30))
        );
        assert_eq!(
            FsmStrategy::ChatTopic.apply(10, 20, Some(30)),
            (10, 10, Some(30))
        );
    }

    #[test]
    fn default_key_builder_matches_upstream_format() {
        let mut key = StorageKey::new(1, 2, 3);
        key.thread_id = Some(4);
        key.destiny = "scene".to_owned();
        let builder = DefaultKeyBuilder::default()
            .with_bot_id(true)
            .with_destiny(true);
        assert_eq!(
            builder.build(&key, Some(KeyPart::State)).unwrap(),
            "fsm:1:2:4:3:scene:state"
        );
    }

    #[tokio::test]
    async fn simple_isolation_serializes_same_key() {
        let isolation = SimpleEventIsolation::default();
        let key = StorageKey::new(1, 2, 3);
        let first = isolation.lock(&key).await.unwrap();
        let pending =
            tokio::time::timeout(std::time::Duration::from_millis(10), isolation.lock(&key)).await;
        assert!(pending.is_err());
        drop(first);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), isolation.lock(&key))
                .await
                .is_ok_and(|result| result.is_ok())
        );
    }

    #[test]
    fn state_group_macro_preserves_aiogram_names() {
        assert_eq!(Form::NAME.full_name().as_deref(), Some("Form:name"));
        assert!(Form::group().contains("Form:age"));
        assert!(State::any().matches(Some("another:state")));
        assert!(!State::default().matches(Some("Form:name")));
    }

    #[tokio::test]
    async fn scene_history_snapshots_and_rolls_back() {
        let context = FsmContext::new(Arc::new(MemoryStorage::default()), StorageKey::new(1, 2, 3));
        context.set_state(Form::NAME.clone()).await.unwrap();
        context
            .set_data(BTreeMap::from([("name".to_owned(), Value::from("Ada"))]))
            .await
            .unwrap();
        let wizard = SceneWizard::new(context.clone());
        wizard.goto(Form::AGE.clone()).await.unwrap();
        assert_eq!(
            context.get_state().await.unwrap().as_deref(),
            Some("Form:age")
        );
        assert_eq!(wizard.back().await.unwrap().as_deref(), Some("Form:name"));
        assert_eq!(
            context.get_value("name").await.unwrap(),
            Some(Value::from("Ada"))
        );
    }

    #[tokio::test]
    async fn scene_registry_routes_hooks_and_after_transitions() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let name_enters = Arc::new(AtomicUsize::new(0));
        let name_leaves = Arc::new(AtomicUsize::new(0));
        let language_enters = Arc::new(AtomicUsize::new(0));
        let registry = SceneRegistry::new();

        let enter_counter = name_enters.clone();
        let leave_counter = name_leaves.clone();
        registry
            .register(
                SceneBuilder::new("scene:name")
                    .message_after(
                        crate::filters::any(),
                        After::goto("scene:language"),
                        |context, scenes| async move {
                            let name = context
                                .message()
                                .and_then(|message| message.text.clone())
                                .unwrap_or_default();
                            scenes
                                .update_data(StateData::from([(
                                    "name".to_owned(),
                                    Value::from(name),
                                )]))
                                .await?;
                            Ok(())
                        },
                    )
                    .action(SceneAction::Enter, "message", move |_, _| {
                        let enter_counter = enter_counter.clone();
                        async move {
                            enter_counter.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    })
                    .action(SceneAction::Leave, "message", move |_, _| {
                        let leave_counter = leave_counter.clone();
                        async move {
                            leave_counter.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    })
                    .build(),
            )
            .unwrap();

        let language_counter = language_enters.clone();
        registry
            .register(
                SceneBuilder::new("scene:language")
                    .message_after(crate::filters::text("back"), After::Back, |_, _| async {
                        Ok(())
                    })
                    .action(SceneAction::Enter, "message", move |_, _| {
                        let language_counter = language_counter.clone();
                        async move {
                            language_counter.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        }
                    })
                    .build(),
            )
            .unwrap();
        assert!(
            registry
                .register(SceneBuilder::new("scene:name").build())
                .is_err()
        );

        let entry_registry = registry.clone();
        let mut entry = crate::Router::new();
        entry.message(crate::filters::command("start"), move |context| {
            let entry_registry = entry_registry.clone();
            async move {
                entry_registry
                    .manager(&context)?
                    .enter(Some("scene:name"))
                    .await
            }
        });

        let storage = Arc::new(MemoryStorage::default());
        let mut dispatcher = crate::Dispatcher::new();
        dispatcher
            .fsm(FsmMiddleware::new(storage.clone()))
            .include_router(entry)
            .include_router(registry.router().unwrap());
        let bot = crate::Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap();
        for (update_id, text) in [(1, "/start"), (2, "Ada"), (3, "back")] {
            let update = serde_json::from_value(serde_json::json!({
                "update_id": update_id,
                "message": {
                    "message_id": update_id,
                    "date": 1,
                    "chat": {"id": 1, "type": "private"},
                    "from": {"id": 2, "is_bot": false, "first_name": "Ada"},
                    "text": text
                }
            }))
            .unwrap();
            assert!(dispatcher.feed_update(bot.clone(), update).await.unwrap());
        }

        let context = FsmContext::new(storage, StorageKey::new(123456, 1, 2));
        assert_eq!(
            context.get_state().await.unwrap().as_deref(),
            Some("scene:name")
        );
        assert_eq!(
            context.get_value("name").await.unwrap(),
            Some(Value::from("Ada"))
        );
        assert_eq!(name_enters.load(Ordering::SeqCst), 2);
        assert_eq!(name_leaves.load(Ordering::SeqCst), 1);
        assert_eq!(language_enters.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "fsm-mongodb")]
    #[test]
    fn mongodb_document_codec_preserves_json_state_data() {
        let data = StateData::from([
            ("name".to_owned(), Value::from("Ada")),
            ("step".to_owned(), Value::from(2)),
            (
                "nested".to_owned(),
                serde_json::json!({"enabled": true, "tags": ["a", "b"]}),
            ),
        ]);
        let document = mongodb::bson::doc! {
            "data": mongodb::bson::to_bson(&data).unwrap()
        };
        assert_eq!(MongoStorage::decode_data(Some(document)).unwrap(), data);
        assert!(MongoStorage::decode_data(None).unwrap().is_empty());
    }

    async fn assert_live_storage_contract(storage: &dyn Storage, key: &StorageKey) {
        storage.clear(key).await.unwrap();
        assert_eq!(storage.get_state(key).await.unwrap(), None);
        assert!(storage.get_data(key).await.unwrap().is_empty());

        storage
            .set_state(key, Some("Form:name".to_owned()))
            .await
            .unwrap();
        storage
            .set_data(
                key,
                StateData::from([
                    ("name".to_owned(), Value::from("Ada")),
                    ("nested".to_owned(), serde_json::json!({"items": [1, 2]})),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(
            storage.get_state(key).await.unwrap().as_deref(),
            Some("Form:name")
        );
        assert_eq!(
            storage.get_value(key, "name").await.unwrap(),
            Some(Value::from("Ada"))
        );
        let updated = storage
            .update_data(
                key,
                StateData::from([
                    ("name".to_owned(), Value::from("Grace")),
                    ("step".to_owned(), Value::from(2)),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(updated["name"], "Grace");
        assert_eq!(updated["step"], 2);
        assert!(updated.contains_key("nested"));

        storage.set_state(key, None).await.unwrap();
        assert_eq!(storage.get_state(key).await.unwrap(), None);
        assert_eq!(
            storage.get_value(key, "name").await.unwrap(),
            Some(Value::from("Grace"))
        );
        storage.set_data(key, StateData::new()).await.unwrap();
        assert!(storage.get_data(key).await.unwrap().is_empty());
        storage.clear(key).await.unwrap();
    }

    #[cfg(feature = "fsm-redis")]
    #[tokio::test]
    #[ignore = "requires AIOGRAM_REDIS_URL and a live Redis service"]
    async fn live_redis_storage_conformance() {
        let url = std::env::var("AIOGRAM_REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379/15".to_owned());
        let storage = RedisStorage::from_url(&url).await.unwrap().key_builder(
            DefaultKeyBuilder::default()
                .with_bot_id(true)
                .with_destiny(true),
        );
        let unique = i64::from(std::process::id());
        let mut key = StorageKey::new(unique, unique + 1, unique + 2);
        key.destiny = "live-contract".to_owned();
        assert_live_storage_contract(&storage, &key).await;

        let ttl_storage = storage
            .clone()
            .state_ttl(std::time::Duration::from_millis(50));
        ttl_storage
            .set_state(&key, Some("temporary".to_owned()))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        assert_eq!(ttl_storage.get_state(&key).await.unwrap(), None);

        let isolation = storage
            .create_isolation()
            .lock_timeout(std::time::Duration::from_millis(60))
            .retry_delay(std::time::Duration::from_millis(5));
        let expired_guard = isolation.lock(&key).await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), isolation.lock(&key))
                .await
                .is_err()
        );
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let current_guard = isolation.lock(&key).await.unwrap();
        drop(expired_guard);
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), isolation.lock(&key))
                .await
                .is_err(),
            "an expired guard must not delete the current owner's lock"
        );
        drop(current_guard);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(250), isolation.lock(&key))
                .await
                .is_ok_and(|result| result.is_ok())
        );
        storage.clear(&key).await.unwrap();
        storage.close().await.unwrap();
    }

    #[cfg(feature = "fsm-mongodb")]
    #[tokio::test]
    #[ignore = "requires AIOGRAM_MONGODB_URL and a live MongoDB service"]
    async fn live_mongodb_storage_conformance() {
        let url = std::env::var("AIOGRAM_MONGODB_URL")
            .unwrap_or_else(|_| "mongodb://127.0.0.1:27017".to_owned());
        let unique = std::process::id();
        let storage = MongoStorage::from_url(&url)
            .await
            .unwrap()
            .namespace("aiogram_fsm_contract", format!("states_{unique}"))
            .key_builder(
                DefaultKeyBuilder::default()
                    .with_bot_id(true)
                    .with_destiny(true),
            );
        let mut key = StorageKey::new(i64::from(unique), 11, 12);
        key.destiny = "live-contract".to_owned();
        assert_live_storage_contract(&storage, &key).await;
        storage.close().await.unwrap();
    }
}
