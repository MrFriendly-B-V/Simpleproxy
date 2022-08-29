use crate::Config;
use actix_web::{web, HttpRequest, HttpResponse};
use anyhow::Result;
use futures_util::StreamExt;
use reqwest::{Client, Response, StatusCode};
use tracing::warn;

pub async fn proxy(
    data: web::Data<Config>,
    req: HttpRequest,
    payload: web::Payload,
) -> HttpResponse {
    let path = req.path();
    let body = match extract_body(payload).await {
        Ok(x) => x,
        Err(e) => {
            warn!("Failed to extract request body: {e}");
            return HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    for (prefix, upstream) in &data.proxy.prefix_routes {
        if path.starts_with(prefix) {
            return reqwest_response_to_actix(
                make_request(req.clone(), body.clone(), upstream).await,
            )
            .await;
        }
    }

    if let Some(upstream) = data.proxy.prefix_routes.get("*") {
        return reqwest_response_to_actix(
            make_request(req.clone(), body.clone(), upstream).await,
        )
        .await;
    }

    HttpResponse::new(StatusCode::NOT_FOUND)
}

/// Extract the request body
async fn extract_body(mut body: web::Payload) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    while let Some(b) = body.next().await {
        let b = b?;
        buf.extend_from_slice(&b);
    }

    Ok(buf)
}

/// Turn a Reqwest response into an Actix response
async fn reqwest_response_to_actix(response: reqwest::Result<Response>) -> HttpResponse {
    let response = match response {
        Ok(x) => x,
        Err(e) => {
            return HttpResponse::build(StatusCode::BAD_GATEWAY).body(e.to_string());
        }
    };

    let mut builder = HttpResponse::build(response.status());
    for (k, v) in response.headers() {
        builder.insert_header((k, v));
    }

    let body = match response.bytes().await {
        Ok(x) => x,
        Err(e) => {
            warn!("Failed to extract response bytes from Reqwest response: {e}");
            return HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    builder.body(body)
}

/// Proxy the request to the provided upstream server.
async fn make_request(
    req: HttpRequest,
    body: Vec<u8>,
    upstream: &str,
) -> reqwest::Result<Response> {
    let client = Client::new();
    let mut req_builder = client.request(
        req.method().clone(),
        format!("{upstream}{}?{}", req.path(), req.query_string()),
    );

    for (k, v) in req.headers() {
        req_builder = req_builder.header(k, v);
    }

    req_builder = req_builder.body(body);
    req_builder.send().await
}
