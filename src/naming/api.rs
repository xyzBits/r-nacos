#![allow(unused_imports, unused_assignments, unused_variables)]
use super::super::utils::{get_bool_from_string, select_option_by_clone};
use super::api_model::{InstanceVO, QueryListResult, ServiceInfoParam};
use super::core::{NamingActor, NamingCmd, NamingResult};
use super::model::{Instance, InstanceUpdateTag, ServiceKey};
use super::ops::ops_api::query_opt_service_list;
use super::NamingUtils;

use actix_web::{
    http::header, middleware, web, App, HttpMessage, HttpRequest, HttpResponse, HttpServer,
    Responder,
};

use actix::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceWebParams {
    pub ip: Option<String>,
    pub port: Option<u32>,
    pub namespace_id: Option<String>,
    pub weight: Option<f32>,
    pub enabled: Option<String>,
    pub healthy: Option<String>,
    pub ephemeral: Option<String>,
    pub metadata: Option<String>,
    pub cluster_name: Option<String>,
    pub service_name: Option<String>,
    pub group_name: Option<String>,
}

impl InstanceWebParams {
    fn select_option(&self, o: &Self) -> Self {
        Self {
            ip: select_option_by_clone(&self.ip, &o.ip),
            port: select_option_by_clone(&self.port, &o.port),
            namespace_id: select_option_by_clone(&self.namespace_id, &o.namespace_id),
            weight: select_option_by_clone(&self.weight, &o.weight),
            enabled: select_option_by_clone(&self.enabled, &o.enabled),
            healthy: select_option_by_clone(&self.healthy, &o.healthy),
            ephemeral: select_option_by_clone(&self.ephemeral, &o.ephemeral),
            metadata: select_option_by_clone(&self.metadata, &o.metadata),
            cluster_name: select_option_by_clone(&self.cluster_name, &o.cluster_name),
            service_name: select_option_by_clone(&self.service_name, &o.service_name),
            group_name: select_option_by_clone(&self.group_name, &o.group_name),
        }
    }

    fn to_instance(self) -> Result<Instance, String> {
        let mut instance = Instance::default();
        instance.ip = Arc::new(self.ip.unwrap());
        instance.port = self.port.unwrap();
        let grouped_name = self.service_name.unwrap();
        if let Some((group_name, service_name)) =
            NamingUtils::split_group_and_serivce_name(&grouped_name)
        {
            instance.service_name = Arc::new(service_name);
            instance.group_name = Arc::new(group_name);
        } else {
            return Err("serivceName is unvaild!".to_owned());
        }
        if let Some(group_name) = self.group_name {
            if group_name.len() > 0 {
                instance.group_name = Arc::new(group_name);
            }
        }
        instance.weight = self.weight.unwrap_or(1f32);
        instance.enabled = get_bool_from_string(&self.enabled, true);
        //instance.healthy= get_bool_from_string(&self.healthy, true);
        instance.healthy = true;
        instance.ephemeral = get_bool_from_string(&self.ephemeral, true);
        instance.cluster_name = NamingUtils::default_cluster(
            self.cluster_name
                .as_ref()
                .unwrap_or(&"".to_owned())
                .to_owned(),
        );
        instance.namespace_id = Arc::new(NamingUtils::default_namespace(
            self.namespace_id
                .as_ref()
                .unwrap_or(&"".to_owned())
                .to_owned(),
        ));
        let metadata_str = self
            .metadata
            .as_ref()
            .unwrap_or(&"{}".to_owned())
            .to_owned();
        match serde_json::from_str::<HashMap<String, String>>(&metadata_str) {
            Ok(metadata) => {
                instance.metadata = Arc::new(metadata);
            }
            Err(_) => {}
        };
        instance.generate_key();
        Ok(instance)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceWebQueryListParams {
    pub namespace_id: Option<String>,
    pub service_name: Option<String>,
    pub group_name: Option<String>,
    pub clusters: Option<String>,
    pub healthy_only: Option<bool>,
    #[serde(rename = "clientIP")]
    pub client_ip: Option<String>,
    pub udp_port: Option<u16>,
}

impl InstanceWebQueryListParams {
    fn to_clusters_key(&self) -> Result<(ServiceKey, String), String> {
        let mut service_name = "".to_owned();
        let mut group_name = "".to_owned();
        let grouped_name = self.service_name.as_ref().unwrap().to_owned();
        if let Some((_group_name, _service_name)) =
            NamingUtils::split_group_and_serivce_name(&grouped_name)
        {
            service_name = _service_name;
            group_name = _group_name;
        } else {
            return Err("serivceName is unvaild!".to_owned());
        }
        if let Some(_group_name) = self.group_name.as_ref() {
            if _group_name.len() > 0 {
                group_name = _group_name.to_owned();
            }
        }
        let namespace_id = NamingUtils::default_namespace(
            self.namespace_id
                .as_ref()
                .unwrap_or(&"".to_owned())
                .to_owned(),
        );
        let key = ServiceKey::new(&namespace_id, &group_name, &service_name);

        /*
        let mut clusters = vec![];
        if let Some(cluster_str) = self.clusters.as_ref() {
            clusters = cluster_str.split(",").into_iter()
                .filter(|e|{e.len()>0}).map(|e|{e.to_owned()}).collect::<Vec<_>>();
        }
        */
        Ok((
            key,
            self.clusters.as_ref().unwrap_or(&"".to_owned()).to_owned(),
        ))
    }

    fn get_addr(&self) -> Option<SocketAddr> {
        if let Some(port) = &self.udp_port {
            if *port == 0u16 {
                return None;
            }
            if let Some(ip_str) = &self.client_ip {
                match ip_str.parse() {
                    Ok(ip) => {
                        return Some(SocketAddr::new(ip, *port));
                    }
                    _ => {}
                }
            }
        }
        None
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BeatRequest {
    pub namespace_id: Option<String>,
    pub service_name: Option<String>,
    pub cluster_name: Option<String>,
    pub group_name: Option<String>,
    pub ephemeral: Option<String>,
    pub beat: Option<String>,
}

impl BeatRequest {
    fn select_option(&self, o: &Self) -> Self {
        Self {
            namespace_id: select_option_by_clone(&self.namespace_id, &o.namespace_id),
            cluster_name: select_option_by_clone(&self.cluster_name, &o.cluster_name),
            service_name: select_option_by_clone(&self.service_name, &o.service_name),
            group_name: select_option_by_clone(&self.group_name, &o.group_name),
            ephemeral: select_option_by_clone(&self.ephemeral, &o.ephemeral),
            beat: select_option_by_clone(&self.beat, &o.beat),
        }
    }

    pub fn to_instance(self) -> Result<Instance, String> {
        let beat = self.beat.as_ref().unwrap();
        let beat_info = serde_json::from_str::<BeatInfo>(beat).unwrap();
        let service_name_option = beat_info.service_name.clone();
        let mut instance = beat_info.to_instance();
        if service_name_option.is_none() {
            let grouped_name = self.service_name.unwrap();
            if let Some((group_name, service_name)) =
                NamingUtils::split_group_and_serivce_name(&grouped_name)
            {
                instance.service_name = Arc::new(service_name);
                instance.group_name = Arc::new(group_name);
            }
            if let Some(group_name) = self.group_name.as_ref() {
                if group_name.len() > 0 {
                    instance.group_name = Arc::new(group_name.to_owned());
                }
            }
        }
        instance.ephemeral = get_bool_from_string(&self.ephemeral, true);
        instance.cluster_name = NamingUtils::default_cluster(
            self.cluster_name
                .as_ref()
                .unwrap_or(&"".to_owned())
                .to_owned(),
        );
        instance.namespace_id = Arc::new(NamingUtils::default_namespace(
            self.namespace_id
                .as_ref()
                .unwrap_or(&"".to_owned())
                .to_owned(),
        ));
        instance.generate_key();
        Ok(instance)
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BeatInfo {
    pub cluster: Option<String>,
    pub ip: Option<String>,
    pub port: Option<u32>,
    pub metadata: Option<HashMap<String, String>>,
    pub period: Option<i64>,
    pub scheduled: Option<bool>,
    pub service_name: Option<String>,
    pub stopped: Option<bool>,
    pub weight: Option<f32>,
}

impl BeatInfo {
    pub fn to_instance(self) -> Instance {
        let mut instance = Instance::default();
        instance.ip = Arc::new(self.ip.unwrap());
        instance.port = self.port.unwrap();
        let grouped_name = self.service_name.as_ref().unwrap().to_owned();
        if let Some((group_name, service_name)) =
            NamingUtils::split_group_and_serivce_name(&grouped_name)
        {
            instance.service_name = Arc::new(service_name);
            instance.group_name = Arc::new(group_name);
        }
        instance.cluster_name = NamingUtils::default_cluster(
            self.cluster.as_ref().unwrap_or(&"".to_owned()).to_owned(),
        );
        if let Some(metadata) = self.metadata {
            instance.metadata = Arc::new(metadata);
        }
        //instance.generate_key();
        instance
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServiceQueryListRequest {
    pub page_no: Option<usize>,
    pub page_size: Option<usize>,
    pub namespace_id: Option<String>,
    pub group_name: Option<String>,
    pub service_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ServiceQueryListResponce {
    pub count: usize,
    pub doms: Vec<Arc<String>>,
}

pub async fn get_instance(
    param: web::Query<InstanceWebParams>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let instance = param.0.to_instance();
    match instance {
        Ok(instance) => match naming_addr.send(NamingCmd::Query(instance)).await {
            Ok(res) => {
                let result: NamingResult = res.unwrap();
                match result {
                    NamingResult::Instance(v) => {
                        let vo = InstanceVO::from_instance(&v);
                        HttpResponse::Ok()
                            .insert_header(header::ContentType(mime::APPLICATION_JSON))
                            .body(serde_json::to_string(&vo).unwrap())
                    }
                    _ => HttpResponse::InternalServerError().body("error"),
                }
            }
            Err(_) => HttpResponse::InternalServerError().body("error"),
        },
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

pub async fn get_instance_list(
    param: web::Query<InstanceWebQueryListParams>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let only_healthy = param.healthy_only.unwrap_or(true);
    let addr = param.get_addr();
    match param.to_clusters_key() {
        Ok((key, clusters)) => {
            match naming_addr
                .send(NamingCmd::QueryListString(
                    key.clone(),
                    clusters,
                    only_healthy,
                    addr,
                ))
                .await
            {
                Ok(res) => {
                    let result: NamingResult = res.unwrap();
                    match result {
                        NamingResult::InstanceListString(v) => HttpResponse::Ok().body(v),
                        _ => HttpResponse::InternalServerError().body("error"),
                    }
                }
                Err(err) => HttpResponse::InternalServerError().body(err.to_string()),
            }
        }
        Err(err) => HttpResponse::InternalServerError().body(err.to_string()),
    }
}

pub async fn add_instance(
    a: web::Query<InstanceWebParams>,
    b: web::Form<InstanceWebParams>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let param = a.select_option(&b);
    let update_tag = InstanceUpdateTag {
        weight: match &param.weight {
            Some(v) => *v != 1.0f32,
            None => false,
        },
        metadata: match &param.metadata {
            Some(v) => !v.is_empty(),
            None => false,
        },
        enabled: false,
        ephemeral: false,
        from_update: false,
    };
    let instance = param.to_instance();
    match instance {
        Ok(instance) => {
            if !instance.check_vaild() {
                HttpResponse::InternalServerError().body("instance check is invalid")
            } else {
                let _ = naming_addr.send(NamingCmd::Update(instance, None)).await;
                HttpResponse::Ok().body("ok")
            }
        }
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

pub async fn update_instance(
    a: web::Query<InstanceWebParams>,
    b: web::Form<InstanceWebParams>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let param = a.select_option(&b);
    let update_tag = InstanceUpdateTag {
        weight: match &param.weight {
            Some(v) => *v != 1.0f32,
            None => false,
        },
        metadata: match &param.metadata {
            //Some(v) => !v.is_empty() && v!="{}",
            Some(v) => !v.is_empty(),
            None => false,
        },
        enabled: match &param.enabled {
            Some(v) => true,
            None => false,
        },
        ephemeral: match &param.ephemeral {
            Some(v) => true,
            None => false,
        },
        from_update: true,
    };
    let instance = param.to_instance();
    match instance {
        Ok(instance) => {
            if !instance.check_vaild() {
                HttpResponse::InternalServerError().body("instance check is invalid")
            } else {
                let _ = naming_addr
                    .send(NamingCmd::Update(instance, Some(update_tag)))
                    .await;
                HttpResponse::Ok().body("ok")
            }
        }
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

pub async fn del_instance(
    a: web::Query<InstanceWebParams>,
    b: web::Form<InstanceWebParams>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let param = a.select_option(&b);
    let instance = param.to_instance();
    match instance {
        Ok(instance) => {
            if !instance.check_vaild() {
                HttpResponse::InternalServerError().body("instance check is invalid")
            } else {
                let _ = naming_addr.send(NamingCmd::Delete(instance)).await;
                HttpResponse::Ok().body("ok")
            }
        }
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

pub async fn beat_instance(
    a: web::Query<BeatRequest>,
    b: web::Form<BeatRequest>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let param = a.select_option(&b);
    let instance = param.to_instance();
    match instance {
        Ok(instance) => {
            if !instance.check_vaild() {
                HttpResponse::InternalServerError().body("instance check is invalid")
            } else {
                let tag = InstanceUpdateTag {
                    weight: false,
                    enabled: false,
                    ephemeral: false,
                    metadata: false,
                    from_update: false,
                };
                let _ = naming_addr
                    .send(NamingCmd::Update(instance, Some(tag)))
                    .await;
                HttpResponse::Ok().body("ok")
            }
        }
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

pub async fn query_service(
    param: web::Query<ServiceQueryListRequest>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    HttpResponse::InternalServerError().body("error,not suport at present")
}

pub async fn update_service(
    param0: web::Form<ServiceInfoParam>,
    param1: web::Query<ServiceInfoParam>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let param = ServiceInfoParam::merge_value(param0.0, param1.0);
    match param.to_service_info() {
        Ok(service_info) => {
            let _ = naming_addr
                .send(NamingCmd::UpdateService(service_info))
                .await;
            HttpResponse::Ok().body("ok")
        }
        Err(err) => HttpResponse::InternalServerError().body(err.to_string()),
    }
}

pub async fn remove_service(
    param0: web::Form<ServiceInfoParam>,
    param1: web::Query<ServiceInfoParam>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let param = ServiceInfoParam::merge_value(param0.0, param1.0);
    match param.to_service_info() {
        Ok(service_info) => {
            let key = service_info.to_service_key();
            match naming_addr.send(NamingCmd::RemoveService(key)).await {
                Ok(res) => {
                    let res: anyhow::Result<NamingResult> = res;
                    match res {
                        Ok(_) => HttpResponse::Ok().body("ok"),
                        Err(err) => HttpResponse::InternalServerError().body(err.to_string()),
                    }
                }
                Err(err) => HttpResponse::InternalServerError().body(err.to_string()),
            }
        }
        Err(err) => HttpResponse::InternalServerError().body(err.to_string()),
    }
}

pub async fn query_service_list(
    param: web::Query<ServiceQueryListRequest>,
    naming_addr: web::Data<Addr<NamingActor>>,
) -> impl Responder {
    let page_size = param.page_size.unwrap_or(0x7fffffff);
    let page_index = param.page_no.unwrap_or(1);
    let namespace_id = NamingUtils::default_namespace(
        param
            .namespace_id
            .as_ref()
            .unwrap_or(&"".to_owned())
            .to_owned(),
    );
    let group = NamingUtils::default_group(
        param
            .group_name
            .as_ref()
            .unwrap_or(&"".to_owned())
            .to_owned(),
    );
    let key = ServiceKey::new(&namespace_id, &group, "");
    match naming_addr
        .send(NamingCmd::QueryServicePage(key, page_size, page_index))
        .await
    {
        Ok(res) => {
            let result: NamingResult = res.unwrap();
            match result {
                NamingResult::ServicePage((c, v)) => {
                    let resp = ServiceQueryListResponce { count: c, doms: v };
                    HttpResponse::Ok().body(serde_json::to_string(&resp).unwrap())
                }
                _ => HttpResponse::InternalServerError().body("error"),
            }
        }
        Err(_) => HttpResponse::InternalServerError().body("error"),
    }
}

pub fn app_config(config: &mut web::ServiceConfig) {
    config.service(
        web::scope("/nacos/v1/ns")
            .service(
                web::resource("/instance")
                    .route(web::get().to(get_instance))
                    .route(web::post().to(add_instance))
                    .route(web::put().to(update_instance))
                    .route(web::delete().to(del_instance)),
            )
            .service(web::resource("/instance/beat").route(web::put().to(beat_instance)))
            .service(web::resource("/instance/list").route(web::get().to(get_instance_list)))
            .service(
                web::resource("/service")
                    .route(web::post().to(update_service))
                    .route(web::put().to(update_service))
                    .route(web::delete().to(remove_service))
                    .route(web::get().to(query_service)),
            )
            .service(web::resource("/service/list").route(web::get().to(query_service_list)))
            //ops
            .service(
                web::resource("/catalog/services").route(web::get().to(query_opt_service_list)),
            ),
    );
}
