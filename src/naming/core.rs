#![allow(
    unused_imports,
    unused_assignments,
    unused_variables,
    unused_mut,
    dead_code
)]

use super::api_model::QueryListResult;
use super::filter::InstanceFilterUtils;
use super::listener::{InnerNamingListener, ListenerItem, NamingListenerCmd};
use super::model::Instance;
use super::model::InstanceKey;
use super::model::InstanceShortKey;
use super::model::InstanceTimeInfo;
use super::model::InstanceUpdateTag;
use super::model::ServiceDetailDto;
use super::model::ServiceInfo;
use super::model::ServiceKey;
use super::model::UpdateInstanceType;
use super::naming_delay_nofity::DelayNotifyActor;
use super::naming_delay_nofity::DelayNotifyCmd;
use super::naming_subscriber::NamingListenerItem;
use super::naming_subscriber::Subscriber;
use super::service::Service;
use super::service::ServiceInfoDto;
use super::service::ServiceMetadata;
use super::service_index::NamespaceIndex;
use super::service_index::ServiceQueryParam;
use super::NamingUtils;
use crate::common::delay_notify;
use crate::common::NamingSysConfig;
use crate::grpc::bistream_manage::BiStreamManage;
use crate::now_millis;
use crate::now_millis_i64;
use crate::utils::gz_encode;
use chrono::Local;
use inner_mem_cache::TimeoutSet;
use serde::{Deserialize, Serialize};
use std::cmp::max;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::LinkedList;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use actix::prelude::*;

//#[derive(Default)]
pub struct NamingActor {
    service_map: HashMap<ServiceKey, Service>,
    last_id: u64,
    listener_addr: Option<Addr<InnerNamingListener>>,
    delay_notify_addr: Option<Addr<DelayNotifyActor>>,
    subscriber: Subscriber,
    sys_config: NamingSysConfig,
    empty_service_set: TimeoutSet<ServiceKey>,
    instance_metadate_set: TimeoutSet<InstanceKey>,
    namespace_index: NamespaceIndex,
    pub(crate) client_instance_set: HashMap<Arc<String>, HashSet<InstanceKey>>,
    //dal_addr: Addr<ServiceDalActor>,
}

impl NamingActor {
    pub fn new(
        listener_addr: Option<Addr<InnerNamingListener>>,
        delay_notify_addr: Option<Addr<DelayNotifyActor>>,
    ) -> Self {
        let mut subscriber = Subscriber::default();
        if let Some(delay_notify) = delay_notify_addr.as_ref() {
            subscriber.set_notify_addr(delay_notify.clone());
        }
        //let dal_addr = SyncArbiter::start(1,||ServiceDalActor::new());
        Self {
            service_map: Default::default(),
            last_id: 0u64,
            listener_addr: listener_addr,
            subscriber: subscriber,
            delay_notify_addr: delay_notify_addr,
            sys_config: NamingSysConfig::new(),
            empty_service_set: Default::default(),
            namespace_index: NamespaceIndex::new(),
            instance_metadate_set: Default::default(),
            client_instance_set: Default::default(),
            //dal_addr,
        }
    }

    pub fn new_and_create() -> Addr<Self> {
        Self::create(move |ctx| {
            //let addr = ctx.address();
            //let listener_addr = InnerNamingListener::new_and_create(period, Some(addr.clone()));
            let delay_notify_addr = DelayNotifyActor::new().start();
            //Self::new(Some(listener_addr),Some(delay_notify_addr))
            Self::new(None, Some(delay_notify_addr))
        })
    }

    pub fn create_at_new_system() -> Addr<Self> {
        let delay_notify_addr = DelayNotifyActor::new().start();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let rt = System::new();
            let addrs = rt.block_on(async { Self::new(None, Some(delay_notify_addr)).start() });
            tx.send(addrs).unwrap();
            rt.run().unwrap();
        });
        let addrs = rx.recv().unwrap();
        addrs
    }

    pub(crate) fn get_service(&mut self, key: &ServiceKey) -> Option<&mut Service> {
        match self.service_map.get_mut(&key) {
            Some(v) => Some(v),
            None => None,
        }
    }

    pub(crate) fn create_empty_service(&mut self, key: &ServiceKey) {
        let ng_service_name = key.service_name.to_owned();
        match self.get_service(key) {
            Some(_) => {}
            None => {
                let mut service = Service::default();
                let current_time = Local::now().timestamp_millis();
                service.service_name = key.service_name.clone();
                service.namespace_id = key.namespace_id.clone();
                service.group_name = key.group_name.clone();
                service.group_service = Arc::new(NamingUtils::get_group_and_service_name(
                    &key.service_name.as_ref(),
                    &key.group_name.as_ref(),
                ));
                service.last_modified_millis = current_time;
                service.recalculate_checksum();
                self.namespace_index.insert_service(key.clone());
                //self.dal_addr.do_send(ServiceDalMsg::AddService(service.get_service_do()));
                self.service_map.insert(key.clone(), service);
                self.empty_service_set.add(
                    now_millis() + self.sys_config.service_time_out_millis,
                    key.clone(),
                );
            }
        }
    }

    pub(crate) fn update_service(&mut self, service_info: ServiceDetailDto) {
        let key = ServiceKey::new_by_arc(
            service_info.namespace_id,
            service_info.group_name,
            service_info.service_name,
        );
        match self.get_service(&key) {
            Some(service) => {
                if let Some(protect_threshold) = service_info.protect_threshold {
                    service.protect_threshold = protect_threshold;
                }
                if let Some(metadata) = service_info.metadata {
                    service.metadata = metadata;
                }
            }
            None => {
                let mut service = Service::default();
                let current_time = Local::now().timestamp_millis();
                service.service_name = key.service_name.clone();
                service.namespace_id = key.namespace_id.clone();
                service.group_name = key.group_name.clone();
                service.group_service = Arc::new(NamingUtils::get_group_and_service_name(
                    &key.service_name.as_ref(),
                    &key.group_name.as_ref(),
                ));
                service.last_modified_millis = current_time;
                if let Some(protect_threshold) = service_info.protect_threshold {
                    service.protect_threshold = protect_threshold;
                }
                if let Some(metadata) = service_info.metadata {
                    service.metadata = metadata;
                }
                service.recalculate_checksum();
                self.namespace_index.insert_service(key.clone());
                //self.dal_addr.do_send(ServiceDalMsg::AddService(service.get_service_do()));
                self.service_map.insert(key.clone(), service);
                self.empty_service_set.add(
                    now_millis() + self.sys_config.service_time_out_millis,
                    key.clone(),
                );
            }
        }
    }

    fn remove_empty_service(&mut self, service_map_key: ServiceKey) -> anyhow::Result<()> {
        if let Some(service) = self.service_map.get(&service_map_key) {
            if service.instance_size <= 0 {
                //控制台发起的不校验过期时间标记
                self.clear_one_empty_service(service_map_key.clone(), 0x7fff_ffff_ffff_ffff);
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "The service has instances,it can't remove!"
                ))
            }
        } else {
            Ok(())
        }
    }

    fn do_notify(&mut self, tag: &UpdateInstanceType, key: ServiceKey) {
        match tag {
            UpdateInstanceType::New => {
                self.subscriber.notify(key);
            }
            UpdateInstanceType::Remove => {
                self.subscriber.notify(key);
            }
            UpdateInstanceType::UpdateValue => {
                self.subscriber.notify(key);
            }
            _ => {}
        }
    }

    /*
    pub(crate) fn add_instance(&mut self,key:&ServiceKey,instance:Instance) -> UpdateInstanceType {
        let service = self.service_map.get_mut(&key).unwrap();
        let tag = service.update_instance(instance,None);
        self.do_notify(&tag, key.clone());
        tag
    }
     */

    pub fn remove_instance(
        &mut self,
        key: &ServiceKey,
        cluster_name: &str,
        instance_id: &InstanceShortKey,
    ) -> UpdateInstanceType {
        let service = self.service_map.get_mut(&key).unwrap();
        let old_instance = service.remove_instance(instance_id);
        let now = now_millis();
        let tag = if let Some(old_instance) = old_instance {
            let short_key = old_instance.get_short_key();
            if service.exist_priority_metadata(&short_key) {
                let instance_key =
                    InstanceKey::new_by_service_key(key, short_key.ip, short_key.port);
                self.instance_metadate_set.add(
                    now + self.sys_config.instance_metadata_time_out_millis,
                    instance_key,
                );
            }
            UpdateInstanceType::Remove
        } else {
            UpdateInstanceType::None
        };
        if service.instance_size <= 0 {
            self.empty_service_set
                .add(now + self.sys_config.service_time_out_millis, key.clone());
        }
        self.do_notify(&tag, key.clone());
        tag
    }

    pub fn update_instance(
        &mut self,
        key: &ServiceKey,
        mut instance: Instance,
        tag: Option<InstanceUpdateTag>,
    ) -> UpdateInstanceType {
        instance.init();
        assert!(instance.check_vaild());
        self.create_empty_service(key);
        //let cluster_name = instance.cluster_name.clone();
        let service = self.service_map.get_mut(&key).unwrap();
        if instance.from_grpc && !instance.client_id.is_empty() {
            let client_id = instance.client_id.clone();
            let key =
                InstanceKey::new_by_service_key(key, instance.ip.clone(), instance.port.to_owned());
            if let Some(set) = self.client_instance_set.get_mut(&client_id) {
                set.insert(key.clone());
            } else {
                let mut set = HashSet::new();
                set.insert(key.clone());
                self.client_instance_set.insert(client_id, set);
            }
        }
        let tag = service.update_instance(instance, tag);
        //change notify
        self.do_notify(&tag, key.clone());
        tag
    }

    pub(crate) fn remove_client_instance(&mut self, client_id: &Arc<String>) {
        if let Some(keys) = self.client_instance_set.remove(client_id) {
            for instance_key in keys {
                let service_key = instance_key.get_service_key();
                let short_key = instance_key.get_short_key();
                self.remove_instance(&service_key, "", &short_key);
            }
        }
    }

    pub fn get_instance(
        &self,
        key: &ServiceKey,
        cluster_name: &str,
        instance_id: &InstanceShortKey,
    ) -> Option<Arc<Instance>> {
        if let Some(service) = self.service_map.get(&key) {
            return service.get_instance(instance_id);
        }
        None
    }

    pub fn get_instance_list(
        &self,
        key: &ServiceKey,
        cluster_str: &str,
        only_healthy: bool,
    ) -> Vec<Arc<Instance>> {
        let cluster_names = NamingUtils::split_filters(&cluster_str);
        if let Some(service) = self.service_map.get(&key) {
            return InstanceFilterUtils::default_instance_filter(
                service.get_instance_list(cluster_names, false, true),
                Some(service.get_metadata()),
                only_healthy,
            );
        }
        vec![]
    }

    pub fn get_instances_and_metadata(
        &self,
        key: &ServiceKey,
        cluster_str: &str,
        only_healthy: bool,
    ) -> (Vec<Arc<Instance>>, Option<ServiceMetadata>) {
        let cluster_names = NamingUtils::split_filters(&cluster_str);
        if let Some(service) = self.service_map.get(&key) {
            return (
                service.get_instance_list(cluster_names, only_healthy, true),
                Some(service.get_metadata()),
            );
        }
        (vec![], None)
    }

    pub fn get_metadata(&self, key: &ServiceKey) -> Option<ServiceMetadata> {
        if let Some(service) = self.service_map.get(&key) {
            Some(service.get_metadata())
        } else {
            None
        }
    }

    pub fn get_instance_map(
        &self,
        key: &ServiceKey,
        cluster_names: Vec<String>,
        only_healthy: bool,
    ) -> HashMap<String, Vec<Arc<Instance>>> {
        let mut map: HashMap<String, Vec<Arc<Instance>>> = HashMap::new();
        if let Some(service) = self.service_map.get(&key) {
            for item in service.get_instance_list(cluster_names, only_healthy, true) {
                if let Some(list) = map.get_mut(&item.cluster_name) {
                    list.push(item)
                } else {
                    map.insert(item.cluster_name.to_owned(), vec![item]);
                }
            }
        }
        map
    }

    pub(crate) fn get_service_info(
        &self,
        key: &ServiceKey,
        cluster_str: String,
        only_healthy: bool,
    ) -> ServiceInfo {
        let mut service_info = ServiceInfo::default();
        //service_info.name = Some(key.get_join_service_name());
        service_info.name = Some(key.service_name.clone());
        service_info.group_name = Some(key.group_name.clone());
        service_info.cache_millis = 10000i64;
        service_info.last_ref_time = now_millis_i64();
        service_info.reach_protection_threshold = false;
        let (hosts, metadata) = self.get_instances_and_metadata(key, &cluster_str, false);
        service_info.hosts = Some(hosts);
        service_info.clusters = Some(cluster_str);
        InstanceFilterUtils::default_service_filter(service_info, metadata, only_healthy)
    }

    pub fn get_instance_list_string(
        &self,
        key: &ServiceKey,
        cluster_str: String,
        only_healthy: bool,
    ) -> String {
        let list = self.get_instance_list(key, &cluster_str, only_healthy);
        QueryListResult::get_instance_list_string(cluster_str, key, list)
    }

    pub fn time_check(&mut self) -> (Vec<InstanceShortKey>, Vec<InstanceShortKey>) {
        let current_time = Local::now().timestamp_millis();
        let healthy_time = current_time - 15000;
        let offline_time = current_time - 30000;
        let mut remove_list = vec![];
        let mut update_list = vec![];
        let mut size = 0;
        let now = now_millis();
        for item in self.service_map.values_mut() {
            let (mut rlist, mut ulist) = item.time_check(healthy_time, offline_time);
            size += rlist.len() + ulist.len();
            remove_list.append(&mut rlist);
            update_list.append(&mut ulist);
            if remove_list.len() > 0 {
                let service_key = item.get_service_key();
                for short_key in &remove_list {
                    if item.exist_priority_metadata(&short_key) {
                        let instance_key = InstanceKey::new_by_service_key(
                            &service_key,
                            short_key.ip.clone(),
                            short_key.port,
                        );
                        self.instance_metadate_set.add(
                            now + self.sys_config.instance_metadata_time_out_millis,
                            instance_key,
                        );
                    }
                }
                if item.instance_size <= 0 {
                    self.empty_service_set.add(
                        now + self.sys_config.service_time_out_millis,
                        item.get_service_key(),
                    );
                }
            }
            if size >= self.sys_config.once_time_check_size {
                break;
            }
        }
        (remove_list, update_list)
    }

    pub fn get_service_list(
        &self,
        page_size: usize,
        page_index: usize,
        key: &ServiceKey,
    ) -> (usize, Vec<Arc<String>>) {
        let offset = page_size * max(page_index - 1, 0);
        let param = ServiceQueryParam {
            offset,
            limit: page_size,
            namespace_id: Some(key.namespace_id.clone()),
            group: Some(key.group_name.clone()),
            ..Default::default()
        };
        let (size, list) = self.namespace_index.query_service_page(&param);
        let service_names = list.into_iter().map(|e| e.service_name).collect::<Vec<_>>();
        (size, service_names)
    }

    pub fn get_service_info_page(&self, param: ServiceQueryParam) -> (usize, Vec<ServiceInfoDto>) {
        let (size, list) = self.namespace_index.query_service_page(&param);
        let mut info_list = Vec::with_capacity(list.len());
        for item in &list {
            if let Some(service) = self.service_map.get(&item) {
                info_list.push(service.get_service_info());
            }
        }
        (size, info_list)
    }

    fn update_listener(
        &mut self,
        key: &ServiceKey,
        cluster_names: &Vec<String>,
        addr: SocketAddr,
        only_healthy: bool,
    ) {
        if let Some(listener_addr) = self.listener_addr.as_ref() {
            let item = ListenerItem::new(cluster_names.clone(), only_healthy, addr);
            let msg = NamingListenerCmd::Add(key.clone(), item);
            listener_addr.do_send(msg);
        }
    }

    fn clear_empty_service(&mut self) {
        //println!("clear_empty_service");
        let now = now_millis();
        for service_map_key in self.empty_service_set.timeout(now) {
            self.clear_one_empty_service(service_map_key, now)
        }
    }

    fn clear_one_empty_service(&mut self, service_map_key: ServiceKey, now: u64) {
        if let Some(service) = self.service_map.get(&service_map_key) {
            if service.instance_size <= 0
                && now - self.sys_config.service_time_out_millis >= service.last_empty_times
            {
                //self.dal_addr.do_send(ServiceDalMsg::DeleteService(service.get_service_do().get_key_param().unwrap()));
                self.namespace_index
                    .remove_service(&service.get_service_key());
                self.service_map.remove(&service_map_key);
                log::info!("clear_empty_service:{:?}", &service_map_key);
            }
        }
    }

    fn clear_timeout_instance_metadata(&mut self) {
        for instance_key in self.instance_metadate_set.timeout(now_millis()) {
            self.clear_one_timeout_instance_metadata(instance_key);
        }
    }

    fn clear_one_timeout_instance_metadata(&mut self, instance_key: InstanceKey) {
        let service_key = instance_key.get_service_key();
        if let Some(service) = self.service_map.get_mut(&service_key) {
            let short_key = instance_key.get_short_key();
            if !service.instances.contains_key(&short_key) {
                service.instance_metadata_map.remove(&short_key);
            }
        }
    }

    pub fn instance_time_out_heartbeat(&self, ctx: &mut actix::Context<Self>) {
        ctx.run_later(Duration::from_millis(2000), |act, ctx| {
            act.clear_empty_service();
            act.clear_timeout_instance_metadata();
            let addr = ctx.address();
            addr.do_send(NamingCmd::PeekListenerTimeout);
            act.instance_time_out_heartbeat(ctx);
        });
    }
}

#[derive(Debug, Message)]
#[rtype(result = "anyhow::Result<NamingResult>")]
pub enum NamingCmd {
    Update(Instance, Option<InstanceUpdateTag>),
    Delete(Instance),
    Query(Instance),
    QueryList(ServiceKey, String, bool, Option<SocketAddr>),
    QueryAllInstanceList(ServiceKey),
    QueryListString(ServiceKey, String, bool, Option<SocketAddr>),
    QueryServiceInfo(ServiceKey, String, bool),
    QueryServicePage(ServiceKey, usize, usize),
    //查询服务实际信息列表
    QueryServiceInfoPage(ServiceQueryParam),
    CreateService(ServiceDetailDto),
    UpdateService(ServiceDetailDto),
    RemoveService(ServiceKey),
    PeekListenerTimeout,
    NotifyListener(ServiceKey, u64),
    SetConnManage(Addr<BiStreamManage>),
    Subscribe(Vec<NamingListenerItem>, Arc<String>),
    RemoveSubscribe(Vec<NamingListenerItem>, Arc<String>),
    RemoveClient(Arc<String>),
    QueryDalAddr,
}

pub enum NamingResult {
    NULL,
    Instance(Arc<Instance>),
    InstanceList(Vec<Arc<Instance>>),
    InstanceListString(String),
    ServiceInfo(ServiceInfo),
    ServicePage((usize, Vec<Arc<String>>)),
    ServiceInfoPage((usize, Vec<ServiceInfoDto>)),
}

impl Actor for NamingActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let msg = NamingListenerCmd::InitNamingActor(ctx.address());
        if let Some(listener_addr) = self.listener_addr.as_ref() {
            listener_addr.do_send(msg);
        }
        if let Some(delay_notify_addr) = self.delay_notify_addr.as_ref() {
            delay_notify_addr.do_send(DelayNotifyCmd::SetNamingAddr(ctx.address()));
        } else {
            let delay_notify_addr = DelayNotifyActor::new().start();
            delay_notify_addr.do_send(DelayNotifyCmd::SetNamingAddr(ctx.address()));
            self.delay_notify_addr = Some(delay_notify_addr);
        }
        log::info!(" NamingActor started");
        self.instance_time_out_heartbeat(ctx);
    }
}

impl Handler<NamingCmd> for NamingActor {
    type Result = anyhow::Result<NamingResult>;

    fn handle(&mut self, msg: NamingCmd, ctx: &mut Context<Self>) -> Self::Result {
        match msg {
            NamingCmd::Update(instance, tag) => {
                self.update_instance(&instance.get_service_key(), instance, tag);
                Ok(NamingResult::NULL)
            }
            NamingCmd::Delete(instance) => {
                self.remove_instance(
                    &instance.get_service_key(),
                    &instance.cluster_name,
                    &instance.get_short_key(),
                );
                Ok(NamingResult::NULL)
            }
            NamingCmd::Query(instance) => {
                if let Some(i) = self.get_instance(
                    &instance.get_service_key(),
                    &instance.cluster_name,
                    &instance.get_short_key(),
                ) {
                    return Ok(NamingResult::Instance(i));
                }
                Ok(NamingResult::NULL)
            }
            NamingCmd::QueryList(service_key, cluster_str, only_healthy, addr) => {
                let cluster_names = NamingUtils::split_filters(&cluster_str);
                if let Some(addr) = addr {
                    self.update_listener(&service_key, &cluster_names, addr, only_healthy);
                }
                let list = self.get_instance_list(&service_key, &cluster_str, only_healthy);
                Ok(NamingResult::InstanceList(list))
            }
            NamingCmd::QueryListString(service_key, cluster_str, only_healthy, addr) => {
                //println!("QUERY_LIST_STRING addr: {:?}",&addr);
                let cluster_names = NamingUtils::split_filters(&cluster_str);
                if let Some(addr) = addr {
                    self.update_listener(&service_key, &cluster_names, addr, only_healthy);
                }
                let data = self.get_instance_list_string(&service_key, cluster_str, only_healthy);
                Ok(NamingResult::InstanceListString(data))
            }
            NamingCmd::QueryServiceInfo(service_key, cluster_str, only_healthy) => {
                let cluster_names = NamingUtils::split_filters(&cluster_str);
                let service_info = self.get_service_info(&service_key, cluster_str, only_healthy);
                Ok(NamingResult::ServiceInfo(service_info))
            }
            NamingCmd::QueryServicePage(service_key, page_size, page_index) => {
                Ok(NamingResult::ServicePage(self.get_service_list(
                    page_size,
                    page_index,
                    &service_key,
                )))
            }
            NamingCmd::QueryServiceInfoPage(param) => Ok(NamingResult::ServiceInfoPage(
                self.get_service_info_page(param),
            )),
            NamingCmd::PeekListenerTimeout => {
                self.time_check();
                //self.notify_check();
                Ok(NamingResult::NULL)
            }
            NamingCmd::NotifyListener(service_key, id) => {
                if let Some(listener_addr) = self.listener_addr.as_ref() {
                    let map = self.get_instance_map(&service_key, vec![], false);
                    //notify listener
                    let msg = NamingListenerCmd::Notify(service_key, "".to_string(), map, id);
                    listener_addr.do_send(msg);
                }
                Ok(NamingResult::NULL)
            }
            NamingCmd::SetConnManage(conn_manage) => {
                if let Some(notify_addr) = self.delay_notify_addr.as_ref() {
                    notify_addr.do_send(DelayNotifyCmd::SetConnManageAddr(conn_manage));
                }
                Ok(NamingResult::NULL)
            }
            NamingCmd::Subscribe(items, client_id) => {
                self.subscriber.add_subscribe(client_id, items.clone());
                //debug
                for item in items {
                    self.subscriber.notify(item.service_key);
                }
                Ok(NamingResult::NULL)
            }
            NamingCmd::RemoveSubscribe(items, client_id) => {
                self.subscriber.remove_subscribe(client_id, items);
                Ok(NamingResult::NULL)
            }
            NamingCmd::RemoveClient(client_id) => {
                self.subscriber.remove_client_subscribe(client_id.clone());
                self.remove_client_instance(&client_id);
                Ok(NamingResult::NULL)
            }
            NamingCmd::QueryDalAddr => {
                //Ok(NamingResult::DalAddr(self.dal_addr.clone()))
                Ok(NamingResult::NULL)
            }
            NamingCmd::CreateService(service_info) => {
                self.update_service(service_info);
                Ok(NamingResult::NULL)
            }
            NamingCmd::UpdateService(service_info) => {
                self.update_service(service_info);
                Ok(NamingResult::NULL)
            }
            NamingCmd::RemoveService(service_key) => {
                self.remove_empty_service(service_key)?;
                Ok(NamingResult::NULL)
            }
            NamingCmd::QueryAllInstanceList(key) => {
                if let Some(service) = self.service_map.get(&key) {
                    Ok(NamingResult::InstanceList(service.get_instance_list(
                        vec![],
                        false,
                        false,
                    )))
                } else {
                    Ok(NamingResult::InstanceList(vec![]))
                }
            }
        }
    }
}

#[actix_rt::test]
async fn query_healthy_instances() {
    use super::*;
    use tokio::net::UdpSocket;
    //let listener_addr = InnerNamingListener::new_and_create(5000, None);
    let mut naming = NamingActor::new(None, None);
    let mut instance = Instance::new("127.0.0.1".to_owned(), 8080);
    instance.namespace_id = Arc::new("public".to_owned());
    instance.service_name = Arc::new("foo".to_owned());
    instance.group_name = Arc::new("DEFUALT".to_owned());
    instance.cluster_name = "DEFUALT".to_owned();
    instance.init();
    let key = instance.get_service_key();
    naming.update_instance(&key, instance, None);
    if let Some(service) = naming.service_map.get_mut(&key) {
        service.protect_threshold = 0.1;
    }

    println!("-------------");
    let items = naming.get_instance_list(&key, "", true);
    assert!(items.len() > 0);
    println!("DEFUALT list:{}", serde_json::to_string(&items).unwrap());
    let items = naming.get_instance_list(&key, "", true);
    assert!(items.len() > 0);
    println!(
        "empty cluster list:{}",
        serde_json::to_string(&items).unwrap()
    );
    tokio::time::sleep(Duration::from_millis(16000)).await;
    naming.time_check();
    println!("-------------");
    let items = naming.get_instance_list(&key, "", false);
    assert!(items.len() > 0);
    println!(
        "empty cluster list:{}",
        serde_json::to_string(&items).unwrap()
    );
    tokio::time::sleep(Duration::from_millis(16000)).await;
    naming.time_check();
    println!("-------------");
    let items = naming.get_instance_list(&key, "", false);
    assert!(items.len() == 0);
    println!(
        "empty cluster list:{}",
        serde_json::to_string(&items).unwrap()
    );
}

#[test]
fn test_add_service() {
    use super::*;
    use tokio::net::UdpSocket;
    let mut naming = NamingActor::new(None, None);
    let service_key = ServiceKey::new("1", "1", "1");
    let service_info = ServiceDetailDto {
        namespace_id: service_key.namespace_id.clone(),
        service_name: service_key.service_name.clone(),
        group_name: service_key.group_name.clone(),
        metadata: Default::default(),
        protect_threshold: Some(0.5),
    };
    assert!(naming.namespace_index.service_size == 0);
    naming.update_service(service_info);
    assert!(naming.namespace_index.service_size == 1);
    naming.remove_empty_service(service_key).unwrap();
    assert!(naming.namespace_index.service_size == 0);
}

#[test]
fn test_remove_has_instance_service() {
    use super::*;
    use tokio::net::UdpSocket;
    let mut naming = NamingActor::new(None, None);
    let mut instance = Instance::new("127.0.0.1".to_owned(), 8080);
    instance.namespace_id = Arc::new("public".to_owned());
    instance.service_name = Arc::new("foo".to_owned());
    instance.group_name = Arc::new("DEFUALT".to_owned());
    instance.cluster_name = "DEFUALT".to_owned();
    instance.init();
    let service_key = instance.get_service_key();
    naming.update_instance(&service_key, instance.clone(), None);
    let service_info = ServiceDetailDto {
        namespace_id: service_key.namespace_id.clone(),
        service_name: service_key.service_name.clone(),
        group_name: service_key.group_name.clone(),
        metadata: Default::default(),
        protect_threshold: Some(0.5),
    };
    assert!(naming.namespace_index.service_size == 1);
    naming.update_service(service_info);
    assert!(naming.namespace_index.service_size == 1);
    assert!(naming.remove_empty_service(service_key.clone()).is_err());
    assert!(naming.namespace_index.service_size == 1);

    naming.remove_instance(
        &service_key,
        &instance.cluster_name,
        &instance.get_short_key(),
    );
    assert!(naming.namespace_index.service_size == 1);
    assert!(naming.remove_empty_service(service_key.clone()).is_ok());
    assert!(naming.namespace_index.service_size == 0);
}
