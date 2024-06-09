use super::model::{Instance, ServiceDetailDto, ServiceKey};
use super::NamingUtils;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryListResult {
    pub name: String,
    pub clusters: String,
    pub cache_millis: u64,
    pub hosts: Vec<InstanceVO>,
    pub last_ref_time: Option<i64>,
    pub checksum: Option<String>,
    #[serde(rename = "useSpecifiedURL")]
    pub use_specified_url: Option<bool>,
    pub env: Option<String>,
    pub protect_threshold: Option<f32>,
    pub reach_local_site_call_threshold: Option<bool>,
    pub dom: Option<Arc<String>>,
    pub metadata: Option<HashMap<String, String>>,
}

impl QueryListResult {
    pub fn get_instance_list_string(
        clusters: String,
        key: &ServiceKey,
        v: Vec<Arc<Instance>>,
    ) -> String {
        let mut result = QueryListResult::default();
        result.name = key.get_join_service_name();
        result.cache_millis = 10000u64;
        let now = Local::now().timestamp_millis();
        result.last_ref_time = Some(now);
        result.checksum = Some(now.to_string());
        result.use_specified_url = Some(false);
        result.clusters = clusters;
        result.env = Some("".to_owned());
        result.hosts = v
            .into_iter()
            .map(|e| InstanceVO::from_instance(&e))
            .collect::<Vec<_>>();
        result.dom = Some(key.service_name.to_owned());
        serde_json::to_string(&result).unwrap()
    }

    pub fn get_ref_instance_list_string(
        clusters: String,
        key: &ServiceKey,
        v: Vec<&Arc<Instance>>,
    ) -> String {
        let mut result = QueryListResult::default();
        result.name = key.get_join_service_name();
        result.cache_millis = 10000u64;
        let now = Local::now().timestamp_millis();
        result.last_ref_time = Some(now - 1000);
        result.checksum = Some(now.to_string());
        result.use_specified_url = Some(false);
        result.clusters = clusters;
        result.env = Some("".to_owned());
        result.hosts = v
            .into_iter()
            .map(|e| InstanceVO::from_instance(&e))
            .collect::<Vec<_>>();
        result.dom = Some(key.service_name.to_owned());
        serde_json::to_string(&result).unwrap()
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstanceVO {
    pub service: Arc<String>,
    pub ip: Arc<String>,
    pub port: u32,
    pub cluster_name: String,
    pub weight: f32,
    pub healthy: bool,
    pub instance_id: Arc<String>,
    pub metadata: Arc<HashMap<String, String>>,
    pub marked: Option<bool>,
    pub enabled: Option<bool>,
    pub service_name: Option<Arc<String>>,
    pub ephemeral: Option<bool>,
}

impl InstanceVO {
    pub fn from_instance(instance: &Instance) -> Self {
        Self {
            service: instance.group_service.clone(),
            ip: instance.ip.clone(),
            port: instance.port,
            cluster_name: instance.cluster_name.to_owned(),
            weight: instance.weight,
            healthy: instance.healthy,
            instance_id: instance.id.clone(),
            metadata: instance.metadata.clone(),
            marked: Some(true),
            enabled: Some(instance.enabled),
            //service_name: Some(instance.service_name.clone()),
            service_name: Some(instance.group_service.clone()),
            ephemeral: Some(instance.ephemeral),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServiceQueryOptListRequest {
    pub page_no: Option<usize>,
    pub page_size: Option<usize>,
    pub namespace_id: Option<String>,
    pub group_name: Option<String>,
    pub service_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfoParam {
    pub namespace_id: Option<String>,
    pub group_name: Option<String>,
    pub service_name: Option<String>,
    pub protect_threshold: Option<f32>,
    pub metadata: Option<String>,
    pub selector: Option<String>,
}

pub fn select_option<T>(a: Option<T>, b: Option<T>) -> Option<T>
where
    T: Clone,
{
    match a {
        Some(v) => Some(v),
        None => b,
    }
}

impl ServiceInfoParam {
    pub(crate) fn merge_value(a: Self, b: Self) -> Self {
        let mut s = Self::default();
        s.namespace_id = select_option(a.namespace_id, b.namespace_id);
        s.group_name = select_option(a.group_name, b.group_name);
        s.service_name = select_option(a.service_name, b.service_name);
        s.protect_threshold = select_option(a.protect_threshold, b.protect_threshold);
        s.metadata = select_option(a.metadata, b.metadata);
        s.selector = select_option(a.selector, b.selector);
        s
    }

    pub(crate) fn to_service_info(self) -> anyhow::Result<ServiceDetailDto> {
        if let Some(service_name) = self.service_name {
            if service_name.is_empty() {
                return Err(anyhow::anyhow!("service_name is vaild"));
            }
            let metadata = if let Some(metadata_str) = self.metadata {
                match serde_json::from_str::<HashMap<String, String>>(&metadata_str) {
                    Ok(metadata) => Some(metadata),
                    Err(_) => None,
                }
            } else {
                None
            };

            Ok(ServiceDetailDto {
                namespace_id: Arc::new(NamingUtils::default_namespace(
                    self.namespace_id.unwrap_or_default(),
                )),
                service_name: Arc::new(service_name),
                group_name: Arc::new(NamingUtils::default_group(
                    self.group_name.unwrap_or_default(),
                )),
                metadata: metadata,
                protect_threshold: self.protect_threshold,
            })
        } else {
            Err(anyhow::anyhow!("service_name is empty"))
        }
    }
}
