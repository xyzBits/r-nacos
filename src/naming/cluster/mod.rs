//distor cluster

use std::{collections::{HashMap, HashSet}, convert::TryFrom, sync::Arc};

use crate::{
    common::appdata::AppShareData,
    naming::core::{NamingCmd, NamingResult},
};

use self::{
    model::{
        NamingRouteRequest, NamingRouterResponse, ProcessRange, SnapshotDataInfo,
        SnapshotForReceive, SyncBatchDataInfo, SyncBatchForReceive,
    },
    node_manage::{NodeManageRequest, NodeManageResponse},
};

pub mod model;
pub mod node_manage;
pub mod route;
pub mod sync_sender;
pub mod instance_delay_notify;

fn get_cluster_id(extend_info: HashMap<String, String>) -> anyhow::Result<u64> {
    if let Some(id_str) = extend_info.get("cluster_id") {
        match id_str.parse() {
            Ok(id) => Ok(id),
            Err(_err) => Err(anyhow::anyhow!("cluster_id can't parse to u64,{}", id_str)),
        }
    } else {
        Err(anyhow::anyhow!("extend_info not found cluster_id"))
    }
}

pub async fn handle_naming_route(
    app: &Arc<AppShareData>,
    req: NamingRouteRequest,
    extend_info: HashMap<String, String>,
) -> anyhow::Result<NamingRouterResponse> {
    match req {
        NamingRouteRequest::Ping(cluster_id) => {
            //更新node_id节点活跃状态
            app.naming_node_manage.active_node(cluster_id);
        }
        NamingRouteRequest::UpdateInstance { instance, tag } => {
            let cmd = NamingCmd::Update(instance, tag);
            let _: NamingResult = app.naming_addr.send(cmd).await??;
        }
        NamingRouteRequest::RemoveInstance { instance } => {
            let cmd = NamingCmd::Delete(instance);
            let _: NamingResult = app.naming_addr.send(cmd).await??;
        }
        NamingRouteRequest::SyncUpdateService { service } => {
            let cluster_id = get_cluster_id(extend_info)?;
            app.naming_addr
                .do_send(NamingCmd::UpdateServiceFromCluster(service));
            app.naming_node_manage.active_node(cluster_id);
        }
        NamingRouteRequest::SyncUpdateInstance { mut instance } => {
            let cluster_id = get_cluster_id(extend_info)?;
            if instance.client_id.is_empty() {
                instance.client_id = Arc::new(format!("{}_G", &cluster_id));
            }
            app.naming_inner_node_manage
                .do_send(NodeManageRequest::AddClientId(
                    cluster_id,
                    instance.client_id.clone(),
                ));
            instance.from_cluster = cluster_id;
            let cmd = NamingCmd::Update(instance, None);
            let _: NamingResult = app.naming_addr.send(cmd).await??;
        }
        NamingRouteRequest::SyncRemoveInstance { mut instance } => {
            let cluster_id = get_cluster_id(extend_info)?;
            app.naming_node_manage.active_node(cluster_id);
            instance.from_cluster = cluster_id;
            let cmd = NamingCmd::Delete(instance);
            let _: NamingResult = app.naming_addr.send(cmd).await??;
        }
        NamingRouteRequest::SyncBatchInstances(data) => {
            let cluster_id = get_cluster_id(extend_info)?;
            let snapshot = SyncBatchDataInfo::from_bytes(&data)?;
            let mut batch_receive = SyncBatchForReceive::try_from(snapshot)?;
            let mut client_sets = HashSet::new();
            for instance in &mut batch_receive.update_instances {
                if instance.client_id.is_empty() {
                    instance.client_id = Arc::new(format!("{}_G", &cluster_id));
                }
                instance.from_cluster = cluster_id;
                client_sets.insert(instance.client_id.clone());
            }
            app.naming_inner_node_manage
                .do_send(NodeManageRequest::AddClientIds(
                    cluster_id,client_sets
                ));
            app.naming_addr
                .do_send(NamingCmd::DeleteBatch(batch_receive.remove_instances));
            app.naming_addr
                .do_send(NamingCmd::UpdateBatch(batch_receive.update_instances));
        }
        NamingRouteRequest::QuerySnapshot { index, len } => {
            //请求 snapshot data
            let cluster_id = get_cluster_id(extend_info)?;
            log::info!("query snapshot from {}", &cluster_id);
            let cmd = NodeManageRequest::QueryOwnerRange(ProcessRange::new(index, len));
            let resp: NodeManageResponse = app.naming_inner_node_manage.send(cmd).await??;
            if let NodeManageResponse::OwnerRange(ranges) = resp {
                let cmd = NamingCmd::QuerySnapshot(ranges);
                let result: NamingResult = app.naming_addr.send(cmd).await??;
                if let NamingResult::Snapshot(snapshot) = result {
                    //发送 snapshot data
                    log::info!("send snapshot to {}", &cluster_id);
                    app.naming_inner_node_manage
                        .do_send(NodeManageRequest::SendSnapshot(cluster_id, snapshot));
                }
            }
            app.naming_node_manage.active_node(cluster_id);
        }
        NamingRouteRequest::Snapshot(data) => {
            let cluster_id = get_cluster_id(extend_info)?;
            //接收snapshot data
            let snapshot = SnapshotDataInfo::from_bytes(&data)?;
            log::info!(
                "receive snapshot from {},instance size:{}",
                &cluster_id,
                snapshot.instances.len()
            );
            let mut snapshot_receive = SnapshotForReceive::try_from(snapshot)?;
            let mut client_sets = HashSet::new();
            for instance in &mut snapshot_receive.instances {
                if instance.client_id.is_empty() {
                    instance.client_id = Arc::new(format!("{}_G", &cluster_id));
                }
                instance.from_cluster = cluster_id;
                client_sets.insert(instance.client_id.clone());
            }
            app.naming_inner_node_manage
                .do_send(NodeManageRequest::AddClientIds(
                    cluster_id,client_sets
                ));
            app.naming_addr
                .do_send(NamingCmd::ReceiveSnapshot(snapshot_receive));
        }
    };
    Ok(NamingRouterResponse::None)
}
