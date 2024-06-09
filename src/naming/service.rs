#![allow(unused_assignments, unused_imports)]

use std::{
    collections::{HashMap, LinkedList},
    hash::Hash,
    sync::{atomic::Ordering, Arc},
};

use actix_web::rt;

use crate::now_millis;

use super::{
    api_model::QueryListResult,
    model::{
        Instance, InstanceShortKey, InstanceTimeInfo, InstanceUpdateTag, ServiceKey,
        UpdateInstanceType,
    },
};

#[derive(Debug, Clone, Default)]
pub struct ServiceMetadata {
    pub protect_threshold: f32,
}

type InstanceMetaData = Arc<HashMap<String, String>>;

#[derive(Debug, Clone, Default)]
pub struct Service {
    pub service_name: Arc<String>,
    pub group_name: Arc<String>,
    pub group_service: Arc<String>,
    pub metadata: HashMap<String, String>,
    pub protect_threshold: f32,
    pub last_modified_millis: i64,
    //pub has_instance:bool,
    pub namespace_id: Arc<String>,
    pub app_name: String,
    pub check_sum: String,
    pub(crate) last_empty_times: u64,
    pub(crate) instance_size: i64,
    pub(crate) healthy_instance_size: i64,
    //pub cluster_map:HashMap<String,Cluster>,
    pub(crate) instances: HashMap<InstanceShortKey, Arc<Instance>>,
    pub(crate) timeinfos: LinkedList<InstanceTimeInfo>,
    pub(crate) instance_metadata_map: HashMap<InstanceShortKey, InstanceMetaData>,
}

impl Service {
    pub(crate) fn recalculate_checksum(&mut self) {
        self.check_sum = "".to_owned();
    }

    /*
    pub(crate) fn remove_instance(&mut self,cluster_name:&str,instance_id:&str) -> UpdateInstanceType {
        if let Some(cluster) = self.cluster_map.get_mut(cluster_name){
            cluster.remove_instance(instance_id);
            return UpdateInstanceType::Remove;
        }
        UpdateInstanceType::None
    }
    */

    pub(crate) fn update_instance(
        &mut self,
        mut instance: Instance,
        update_tag: Option<InstanceUpdateTag>,
    ) -> UpdateInstanceType {
        /*
        if instance.service_name=="service-consumer" {
            //println!("service-consumer update_instance {:?}",&instance);
        }
        */
        instance.namespace_id = self.namespace_id.clone();
        instance.group_name = self.group_name.clone();
        instance.service_name = self.service_name.clone();
        instance.group_service = self.group_service.clone();
        let key = instance.get_short_key();
        //let mut update_mark = true;
        let mut rtype = UpdateInstanceType::None;
        let short_key = instance.get_short_key();
        let old_instance = self.instances.get(&key);
        if let Some(old_instance) = old_instance {
            if !old_instance.healthy {
                self.healthy_instance_size += 1;
            }
            rtype = UpdateInstanceType::UpdateValue;
            if let Some(update_tag) = update_tag {
                if !update_tag.is_none() {
                    if !update_tag.enabled {
                        instance.enabled = old_instance.enabled.to_owned();
                    }
                    if !update_tag.ephemeral {
                        instance.ephemeral = old_instance.ephemeral.to_owned();
                    }
                    if !update_tag.weight {
                        instance.weight = old_instance.weight.to_owned();
                    }
                    if !update_tag.metadata {
                        instance.metadata = old_instance.metadata.clone();
                    } else {
                        if update_tag.from_update {
                            //从控制台设置的metadata
                            self.instance_metadata_map
                                .insert(short_key, instance.metadata.clone());
                        } else {
                            if let Some(priority_metadata) =
                                self.instance_metadata_map.get(&short_key)
                            {
                                //sdk更新尝试使用高优先级metadata
                                instance.metadata = priority_metadata.clone();
                            }
                        }
                    }
                } else {
                    //不更新
                    instance.enabled = old_instance.enabled.to_owned();
                    instance.ephemeral = old_instance.ephemeral.to_owned();
                    instance.weight = old_instance.weight.to_owned();
                    instance.metadata = old_instance.metadata.clone();
                    rtype = UpdateInstanceType::UpdateTime;
                }
            }
            if old_instance.from_grpc {
                instance.from_grpc = old_instance.from_grpc;
            }
        } else {
            //新增的尝试使用高优先级metadata
            if let Some(priority_metadata) = self.instance_metadata_map.get(&short_key) {
                instance.metadata = priority_metadata.clone();
            }
            self.instance_size += 1;
            self.healthy_instance_size += 1;
            rtype = UpdateInstanceType::New;
        }
        let new_instance = Arc::new(instance);
        //grpc 不走timecheck
        if !new_instance.from_grpc {
            let time_info = new_instance.get_time_info();
            self.update_timeinfos(time_info);
        }
        self.instances.insert(key, new_instance);
        /*
        if update_mark {
            self.update_timeinfos(time_info);
        }
        */
        rtype
    }

    pub(crate) fn update_timeinfos(&mut self, time_info: InstanceTimeInfo) {
        for item in &mut self.timeinfos {
            if item.instance_id == time_info.instance_id {
                item.enable = false;
            }
        }
        self.timeinfos.push_back(time_info);
    }

    pub(crate) fn time_check(
        &mut self,
        healthy_time: i64,
        offline_time: i64,
    ) -> (Vec<InstanceShortKey>, Vec<InstanceShortKey>) {
        assert!(healthy_time >= offline_time);
        let mut i = 0;
        let t = self.timeinfos.iter();
        let mut remove_list = vec![];
        let mut update_list = vec![];
        let mut remove_index = 0;
        for item in t {
            i += 1;
            if !item.enable {
                continue;
            }
            if item.time <= healthy_time {
                if item.time <= offline_time {
                    remove_list.push(item.instance_id.clone());
                    remove_index = i;
                } else {
                    update_list.push(item.instance_id.clone());
                }
            } else {
                break;
            }
        }
        self.timeinfos = self.timeinfos.split_off(remove_index);
        for item in &remove_list {
            self.remove_instance(&item);
        }
        for item in &update_list {
            self.update_instance_healthy_unvaild(&item);
        }
        (remove_list, update_list)
    }

    pub(crate) fn remove_instance(
        &mut self,
        instance_key: &InstanceShortKey,
    ) -> Option<Arc<Instance>> {
        if let Some(old) = self.instances.remove(instance_key) {
            self.instance_size -= 1;
            if self.instance_size == 0 {
                self.last_empty_times = now_millis();
            }
            if old.healthy {
                self.healthy_instance_size -= 1;
            }
            Some(old)
        } else {
            None
        }
    }

    pub(crate) fn update_instance_healthy_unvaild(&mut self, instance_id: &InstanceShortKey) {
        if let Some(i) = self.instances.remove(instance_id) {
            if i.healthy {
                self.healthy_instance_size -= 1;
            }
            let mut i = i.as_ref().clone();
            i.healthy = false;
            self.instances.insert(instance_id.clone(), Arc::new(i));
        }
    }

    pub(crate) fn get_instance(&self, instance_key: &InstanceShortKey) -> Option<Arc<Instance>> {
        self.instances
            .get(instance_key)
            .map_or(None, |i| Some(i.clone()))
    }

    pub(crate) fn get_all_instances(
        &self,
        only_healthy: bool,
        only_enable: bool,
    ) -> Vec<Arc<Instance>> {
        self.instances
            .values()
            .filter(|x| (x.enabled || !only_enable) && (x.healthy || !only_healthy))
            .map(|x| x.clone())
            .collect::<Vec<_>>()
    }

    /*
    pub(crate) fn notify_listener(&mut self,cluster_name:&str,updateType:UpdateInstanceType) -> UpdateInstanceType {
        if match updateType {
            UpdateInstanceType::New =>{false},
            UpdateInstanceType::Remove =>{false},
            UpdateInstanceType::UpdateValue =>{false},
            _ => {true}
        }{
            return updateType;
        }
        updateType
    }
    */

    pub(crate) fn get_instance_list(
        &self,
        _cluster_names: Vec<String>,
        only_healthy: bool,
        only_enable: bool,
    ) -> Vec<Arc<Instance>> {
        self.get_all_instances(only_healthy, only_enable)
    }

    pub fn get_service_key(&self) -> ServiceKey {
        ServiceKey::new_by_arc(
            self.namespace_id.clone(),
            self.group_name.clone(),
            self.service_name.clone(),
        )
    }

    pub fn get_metadata(&self) -> ServiceMetadata {
        ServiceMetadata {
            protect_threshold: self.protect_threshold,
        }
    }

    pub fn get_service_info(&self) -> ServiceInfoDto {
        ServiceInfoDto {
            service_name: self.service_name.clone(),
            group_name: self.group_name.clone(),
            instance_size: self.instance_size,
            healthy_instance_size: self.healthy_instance_size,
            cluster_count: 0,
            trigger_flag: false,
            metadata: Some(self.metadata.clone()),
            protect_threshold: Some(self.protect_threshold),
        }
    }

    pub(crate) fn exist_priority_metadata(&self, instance_key: &InstanceShortKey) -> bool {
        self.instance_metadata_map.contains_key(&instance_key)
    }
}

#[derive(Debug, Default, Clone)]
pub struct ServiceInfoDto {
    pub service_name: Arc<String>,
    pub group_name: Arc<String>,
    pub instance_size: i64,
    pub healthy_instance_size: i64,
    pub cluster_count: i64,
    pub trigger_flag: bool,
    pub metadata: Option<HashMap<String, String>>,
    pub protect_threshold: Option<f32>,
}
