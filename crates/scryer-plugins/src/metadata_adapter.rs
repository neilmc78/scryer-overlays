//! Metadata-provider plugins (a local addition): the client over one loaded
//! component and the provider that holds every installed one.
//!
//! A metadata provider has no per-instance configs. It sees the installation
//! settings its descriptor declares (an OMDb API key, a daily request limit)
//! and keeps its own cache and budget in plugin state.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use scryer_application::metadata_plugins::{MetadataPluginProvider, MetadataProviderClient};
use scryer_application::{AppError, AppResult, RuntimePluginLoad};
use scryer_plugin_sdk::command::{
    PluginCommand, PluginCommandRequest, PluginCommandResult, PluginMetadataCommand,
    PluginMetadataCommandResult,
};
use scryer_plugin_sdk::{
    MetadataMediaKind, MetadataPluginHealthRequest, MetadataPluginHealthResponse,
    MetadataPluginLookupRequest, MetadataPluginLookupResponse, MetadataPluginSearchRequest,
    MetadataPluginSearchResponse, PluginDescriptor, PluginResult,
};
use tracing::warn;

use crate::loader::{
    LoadedPlugin, LoadedPluginRecord, PluginLoadSource, allowed_hosts_for_descriptor,
    insert_loaded_plugin, operator_egress_policy_for_descriptor, remove_loaded_plugin,
    validate_descriptor_for_type,
};
use crate::runtime_backing::{PluginInstanceSpec, PluginRuntimeBacking};
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::{MetadataComponentInvocation, process_metadata_component};

const METADATA_PLUGIN_TYPE: &str = "metadata_provider";

/// Wall-clock budget for one metadata invocation: one or two HTTP requests.
const METADATA_PLUGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// One `scryer:metadata/metadata-provider@1.0.0` component with its
/// installation settings bound.
pub struct WasmMetadataClient {
    spec: PluginInstanceSpec,
    descriptor: PluginDescriptor,
}

impl WasmMetadataClient {
    async fn invoke(
        &self,
        command: PluginMetadataCommand,
        operation: &'static str,
    ) -> AppResult<PluginMetadataCommandResult> {
        let response = process_metadata_component(
            &self.spec,
            &PluginCommandRequest::new(PluginCommand::Metadata(command)),
            MetadataComponentInvocation {
                plugin_id: &self.descriptor.id,
                plugin_version: &self.descriptor.version,
                operation,
            },
        )
        .await?;
        match response.response {
            PluginCommandResult::Metadata(result) => Ok(result),
            _ => Err(AppError::Repository(format!(
                "metadata plugin {} returned a response for another plugin family",
                self.descriptor.id
            ))),
        }
    }

    fn wrong_operation(&self, operation: &str) -> AppError {
        AppError::Repository(format!(
            "metadata plugin {} answered a {operation} command with another operation",
            self.descriptor.id
        ))
    }
}

#[async_trait]
impl MetadataProviderClient for WasmMetadataClient {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    async fn lookup(
        &self,
        kind: MetadataMediaKind,
        imdb_id: &str,
    ) -> AppResult<PluginResult<MetadataPluginLookupResponse>> {
        match self
            .invoke(
                PluginMetadataCommand::Lookup(MetadataPluginLookupRequest {
                    kind,
                    imdb_id: imdb_id.to_string(),
                }),
                "metadata_lookup",
            )
            .await?
        {
            PluginMetadataCommandResult::Lookup(result) => Ok(result),
            _ => Err(self.wrong_operation("lookup")),
        }
    }

    async fn search(
        &self,
        kind: MetadataMediaKind,
        query: &str,
        year: Option<i32>,
        limit: u32,
    ) -> AppResult<PluginResult<MetadataPluginSearchResponse>> {
        match self
            .invoke(
                PluginMetadataCommand::Search(MetadataPluginSearchRequest {
                    kind,
                    query: query.to_string(),
                    year,
                    limit,
                }),
                "metadata_search",
            )
            .await?
        {
            PluginMetadataCommandResult::Search(result) => Ok(result),
            _ => Err(self.wrong_operation("search")),
        }
    }

    async fn health(&self) -> AppResult<PluginResult<MetadataPluginHealthResponse>> {
        match self
            .invoke(
                PluginMetadataCommand::Health(MetadataPluginHealthRequest {}),
                "metadata_health",
            )
            .await?
        {
            PluginMetadataCommandResult::Health(result) => Ok(result),
            _ => Err(self.wrong_operation("health")),
        }
    }
}

/// The installed metadata providers, keyed by provider type with aliases.
///
/// Clients are built once per load, not per call: each owns the command host
/// that holds the plugin's state, so a provider's answer cache and request
/// budget last until the plugin is next installed, reconfigured or reloaded.
pub struct WasmMetadataPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
    clients: Vec<Arc<dyn MetadataProviderClient>>,
}

impl WasmMetadataPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
            clients: Vec::new(),
        }
    }

    fn rebuild_clients(&mut self) {
        let mut loaded = self.plugins.values().collect::<Vec<_>>();
        loaded.sort_by(|left, right| {
            left.descriptor
                .provider_type()
                .cmp(right.descriptor.provider_type())
        });
        self.clients = loaded
            .into_iter()
            .filter_map(Self::build_client)
            .map(|client| Arc::new(client) as Arc<dyn MetadataProviderClient>)
            .collect();
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !validate_descriptor_for_type(
            &plugin.descriptor,
            Some(METADATA_PLUGIN_TYPE),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("metadata provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(
            LoadedPlugin::from_owned(plugin.descriptor, plugin.wasm_bytes)
                .with_settings(plugin.settings),
        ))
    }

    pub fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
                self.rebuild_clients();
            }
            Err(error) => warn!(error = %error, "failed to load runtime metadata provider plugin"),
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self.rebuild_clients();
        self
    }

    fn build_client(loaded: &LoadedPlugin) -> Option<WasmMetadataClient> {
        let provider_type = loaded.descriptor.provider_type().to_string();
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    error = %error,
                    "failed to materialize metadata provider plugin bytes"
                );
                return None;
            }
        };
        match PluginRuntimeBacking::for_artifact(&loaded.descriptor, &wasm_bytes) {
            Ok(PluginRuntimeBacking::Metadata) => {}
            Ok(_) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    "metadata provider selected a runtime that is not valid for this descriptor family"
                );
                return None;
            }
            Err(error) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    error = %error,
                    "metadata provider has an invalid runtime marker"
                );
                return None;
            }
        }

        let mut plugin_config = BTreeMap::new();
        let base_url = loaded
            .descriptor
            .metadata_provider()
            .and_then(|provider| provider.default_base_url.as_deref());
        let allowed_hosts = allowed_hosts_for_descriptor(&loaded.descriptor, base_url, Some("{}"));
        let egress_policy = operator_egress_policy_for_descriptor(None, Some("{}"));
        loaded.bind_settings(&mut plugin_config);
        let command_host = CommandHost::with_archive_provider(
            loaded.descriptor.id.clone(),
            plugin_config,
            allowed_hosts,
            egress_policy,
            METADATA_PLUGIN_TIMEOUT,
            None,
            None,
        );
        Some(WasmMetadataClient {
            spec: PluginInstanceSpec {
                wasm: Arc::new(wasm_bytes),
                // Metadata providers have no filesystem authority.
                preopens: Vec::new(),
                timeout: METADATA_PLUGIN_TIMEOUT,
                memory_max_bytes: None,
                command_host,
            },
            descriptor: loaded.descriptor.clone(),
        })
    }
}

impl MetadataPluginProvider for WasmMetadataPluginProvider {
    fn clients(&self) -> Vec<Arc<dyn MetadataProviderClient>> {
        self.clients.clone()
    }
}

/// The reloadable wrapper the application holds.
pub struct DynamicMetadataPluginProvider {
    inner: std::sync::RwLock<WasmMetadataPluginProvider>,
}

impl DynamicMetadataPluginProvider {
    pub fn new(provider: WasmMetadataPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
        }
    }

    fn replace_with(
        &self,
        update: impl FnOnce(WasmMetadataPluginProvider) -> WasmMetadataPluginProvider,
    ) {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = std::mem::replace(&mut *guard, WasmMetadataPluginProvider::empty());
        *guard = update(current);
    }
}

impl MetadataPluginProvider for DynamicMetadataPluginProvider {
    fn clients(&self) -> Vec<Arc<dyn MetadataProviderClient>> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clients()
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        self.replace_with(|current| current.with_runtime_plugin(plugin));
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        self.replace_with(|current| current.without_provider_type(provider_type));
        Ok(())
    }

    fn reload_runtime_plugins(&self, runtime_plugins: &[RuntimePluginLoad]) -> Result<(), String> {
        let rebuilt = build_metadata_plugin_provider_from_runtime_plugins(runtime_plugins);
        *self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = rebuilt;
        Ok(())
    }
}

pub fn build_metadata_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
) -> WasmMetadataPluginProvider {
    let mut provider = WasmMetadataPluginProvider::empty();
    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }
    provider
}
