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
    let host = match req.headers().get("host") {
        Some(x) => match x.to_str() {
            Ok(x) => x,
            Err(_) => return HttpResponse::build(StatusCode::BAD_REQUEST).body("Invalid header 'Host'"),
        },
        None => return HttpResponse::build(StatusCode::BAD_REQUEST).body("Missing header 'Host'"),
    };

    let body = match extract_body(payload).await {
        Ok(x) => x,
        Err(e) => {
            warn!("Failed to extract request body: {e}");
            return HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    let possible_routes = data.proxy.routes.iter()
        .filter(|x| {
            // Check for a route matching the host
            if let Some(route_host) = &x.host {
                if route_host.eq(&host) {
                    return true;
                }
            }

            // Check for a route matching the path prefix
            if let Some(route_path_prefix) = &x.path_prefix {
                if path.starts_with(route_path_prefix.as_str()) {
                    return true;
                }
            }

            false
        })
        .collect::<Vec<_>>();
    let route = possible_routes.first();

    let route = match route {
        Some(x) => x,
        None => {
            // Check if there's a default route configured
            let default_routes = data.proxy.routes.iter().filter(|x| x.default.eq(&Some(true))).collect::<Vec<_>>();
            match default_routes.first() {
                Some(x) => x.clone(),
                None => return HttpResponse::NotFound().finish(),
            }
        },
    };

    reqwest_response_to_actix(make_request(req.clone(), body.clone(), &route.upstream).await).await
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
