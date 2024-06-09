use crate::{config::config::ConfigActor, naming::core::NamingActor};

use self::{
    config_change_batch_listen::ConfigChangeBatchListenRequestHandler,
    config_publish::ConfigPublishRequestHandler, config_query::ConfigQueryRequestHandler,
    config_remove::ConfigRemoveRequestHandler, naming_batch_instance::BatchInstanceRequestHandler,
    naming_instance::InstanceRequestHandler, naming_service_query::ServiceQueryRequestHandler,
    naming_subscribe_service::SubscribeServiceRequestHandler,
};

use super::{
    api_model::{BaseResponse, ServerCheckResponse, SUCCESS_CODE},
    nacos_proto::Payload,
    PayloadHandler, PayloadUtils, RequestMeta,
};
use actix::Addr;
use async_trait::async_trait;

pub mod config_change_batch_listen;
pub mod config_publish;
pub mod config_query;
pub mod config_remove;

pub mod converter;
pub mod naming_batch_instance;
pub mod naming_instance;
pub mod naming_service_query;
pub mod naming_subscribe_service;

#[derive(Default)]
pub struct InvokerHandler {
    handlers: Vec<(String, Box<dyn PayloadHandler + Send + Sync + 'static>)>,
}
pub struct HealthCheckRequestHandler {}

impl InvokerHandler {
    pub fn new() -> Self {
        let mut this = Self {
            handlers: Default::default(),
        };
        this.add_handler("HealthCheckRequest", Box::new(HealthCheckRequestHandler {}));
        this
    }

    pub fn add_handler(
        &mut self,
        url: &str,
        handler: Box<dyn PayloadHandler + Send + Sync + 'static>,
    ) {
        self.handlers.push((url.to_owned(), handler));
    }

    pub fn match_handler<'a>(
        &'a self,
        url: &str,
    ) -> Option<&'a Box<dyn PayloadHandler + Send + Sync + 'static>> {
        for (t, h) in &self.handlers {
            if t == url {
                return Some(h);
            }
        }
        None
    }

    pub fn add_config_handler(&mut self, config_addr: &Addr<ConfigActor>) {
        self.add_handler(
            "ConfigQueryRequest",
            Box::new(ConfigQueryRequestHandler::new(config_addr.clone())),
        );
        self.add_handler(
            "ConfigPublishRequest",
            Box::new(ConfigPublishRequestHandler::new(config_addr.clone())),
        );
        self.add_handler(
            "ConfigRemoveRequest",
            Box::new(ConfigRemoveRequestHandler::new(config_addr.clone())),
        );
        self.add_handler(
            "ConfigBatchListenRequest",
            Box::new(ConfigChangeBatchListenRequestHandler::new(
                config_addr.clone(),
            )),
        );
    }

    pub fn add_naming_handler(&mut self, naming_addr: &Addr<NamingActor>) {
        self.add_handler(
            "InstanceRequest",
            Box::new(InstanceRequestHandler::new(naming_addr.clone())),
        );
        self.add_handler(
            "BatchInstanceRequest",
            Box::new(BatchInstanceRequestHandler::new(naming_addr.clone())),
        );
        self.add_handler(
            "SubscribeServiceRequest",
            Box::new(SubscribeServiceRequestHandler::new(naming_addr.clone())),
        );
        self.add_handler(
            "ServiceQueryRequest",
            Box::new(ServiceQueryRequestHandler::new(naming_addr.clone())),
        );
    }
}

#[async_trait]
impl PayloadHandler for InvokerHandler {
    async fn handle(
        &self,
        request_payload: super::nacos_proto::Payload,
        request_meta: RequestMeta,
    ) -> anyhow::Result<Payload> {
        if let Some(url) = PayloadUtils::get_payload_type(&request_payload) {
            if "ServerCheckRequest" == url {
                let mut response = ServerCheckResponse::default();
                response.result_code = SUCCESS_CODE;
                response.connection_id = Some(request_meta.connection_id.as_ref().to_owned());
                return Ok(PayloadUtils::build_payload(
                    "ServerCheckResponse",
                    serde_json::to_string(&response)?,
                ));
            }
            //println!("InvokerHandler type:{}",url);
            if let Some(handler) = self.match_handler(url) {
                return handler.handle(request_payload, request_meta).await;
            }
            log::warn!("InvokerHandler not fund handler,type:{}", url);
            return Ok(PayloadUtils::build_error_payload(
                302u16,
                format!("{} RequestHandler Not Found", url),
            ));
        }
        Ok(PayloadUtils::build_error_payload(
            302u16,
            "empty type url".to_owned(),
        ))
    }
}

#[async_trait]
impl PayloadHandler for HealthCheckRequestHandler {
    async fn handle(
        &self,
        _request_payload: super::nacos_proto::Payload,
        _request_meta: RequestMeta,
    ) -> anyhow::Result<Payload> {
        println!("HealthCheckRequest");
        let response = BaseResponse::build_success_response();
        return Ok(PayloadUtils::build_payload(
            "HealthCheckResponse",
            serde_json::to_string(&response)?,
        ));
    }
}
