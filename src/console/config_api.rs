#![allow(unused_imports)]

use std::io::{self, Write};
use std::sync::Arc;

use actix_multipart::form::tempfile::TempFile;
use actix_multipart::form::text::Text;
use actix_multipart::form::MultipartForm;
use actix_multipart::Multipart;
use actix_web::{http::header, web, Error, HttpRequest, HttpResponse, Responder};

use crate::config::config::{ConfigActor, ConfigCmd, ConfigKey, ConfigResult};
use crate::config::ConfigUtils;
use crate::console::model::config_model::{
    OpsConfigOptQueryListResponse, OpsConfigQueryListRequest,
};
use actix::prelude::Addr;
use tokio_stream::StreamExt;
use uuid::Uuid;
use zip::ZipArchive;

use super::model::config_model::OpsConfigImportInfo;

pub async fn query_config_list(
    request: web::Query<OpsConfigQueryListRequest>,
    config_addr: web::Data<Addr<ConfigActor>>,
) -> impl Responder {
    let cmd = ConfigCmd::QueryPageInfo(Box::new(request.0.to_param().unwrap()));
    match config_addr.send(cmd).await {
        Ok(res) => {
            let r: ConfigResult = res.unwrap();
            match r {
                ConfigResult::ConfigInfoPage(size, list) => {
                    let response = OpsConfigOptQueryListResponse {
                        count: size as u64,
                        list,
                    };
                    let v = serde_json::to_string(&response).unwrap();
                    HttpResponse::Ok()
                        .insert_header(header::ContentType(mime::APPLICATION_JSON))
                        .body(v)
                }
                _ => HttpResponse::InternalServerError().body("config result error"),
            }
        }
        Err(err) => HttpResponse::InternalServerError().body(err.to_string()),
    }
}

#[derive(Debug, MultipartForm)]
pub struct UploadForm {
    #[multipart(rename = "tenant")]
    pub tenant: Option<Text<String>>,
    #[multipart(rename = "file")]
    pub files: Vec<TempFile>,
}

pub async fn import_config(
    req: HttpRequest,
    MultipartForm(form): MultipartForm<UploadForm>,
    config_addr: web::Data<Addr<ConfigActor>>,
) -> Result<impl Responder, Error> {
    let tenant = Arc::new(ConfigUtils::default_tenant(
        match req.headers().get("tenant") {
            Some(v) => String::from_utf8_lossy(v.as_bytes()).to_string(),
            None => "".to_owned(),
        },
    ));
    //let tenant = Arc::new(ConfigUtils::default_tenant(config_info.0.tenant.unwrap_or_default()));
    for f in form.files {
        match zip::ZipArchive::new(f.file) {
            Ok(mut archive) => {
                for i in 0..archive.len() {
                    let mut file = archive.by_index(i).unwrap();
                    /*
                    let filepath = match file.enclosed_name() {
                        Some(path) => path,
                        None => continue,
                    };
                    */
                    let filename = file.name();
                    if !(*filename).ends_with('/') {
                        let parts = filename.split('/').into_iter().collect::<Vec<_>>();
                        if parts.len() != 2 {
                            continue;
                        }
                        assert!(parts.len() == 2);
                        let config_key = ConfigKey::new_by_arc(
                            Arc::new(parts[1].to_owned()),
                            Arc::new(parts[0].to_owned()),
                            tenant.clone(),
                        );
                        let value = match io::read_to_string(&mut file) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        //println!("update load, {:?}:{}",&config_key,&value);
                        config_addr.do_send(ConfigCmd::ADD(config_key, Arc::new(value)));
                    }
                }
            }
            Err(_) => todo!(),
        }
    }
    Ok(HttpResponse::Ok())
}
