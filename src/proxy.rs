use std::borrow::Cow;
use std::collections::HashMap;
use crate::Config;
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web::http::header::{HeaderName, HeaderValue};
use anyhow::Result;
use futures_util::StreamExt;
use reqwest::{Client, Response, StatusCode};
use tracing::{warn, instrument, debug};
use crate::config::{ProxyConfig, Route};

#[instrument(skip(data, req, payload))]
pub async fn proxy(
    data: web::Data<Config>,
    req: HttpRequest,
    payload: web::Payload
) -> HttpResponse {
    let path = req.path();

    let host = match req.headers().get("host") {
        Some(x) => match x.to_str() {
            Ok(x) => x,
            Err(_) => return HttpResponse::build(StatusCode::BAD_REQUEST).body("Invalid header 'Host'"),
        },
        None => {
            // HTTP 2 does not supply the Host header
            match req.uri().host() {
                Some(x) => x,
                None => return HttpResponse::build(StatusCode::BAD_REQUEST).body("Missing header 'Host' (HTTP/1.1) or the Host portion of the URI (HTTP/2)")
            }
        },
    };

    let body = match extract_body(payload).await {
        Ok(x) => x,
        Err(e) => {
            warn!("Failed to extract request body: {e}");
            return HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    let possible_routes = data.routes.iter()
        .filter(|x| {
            // Check for a route matching the host
            if let Some(route_host) = &x.host {
                if route_host.eq(&host) {
                    // Even if the host matches, if a path is provided, the path must match too
                    if let Some(route_path_prefix) = &x.path_prefix {
                        if path.starts_with(route_path_prefix.as_str()) {
                            return true;
                        }
                    } else {
                        return true;
                    }
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
            let default_routes = data.routes.iter().filter(|x| x.default.eq(&Some(true))).collect::<Vec<_>>();
            match default_routes.first() {
                Some(x) => x.clone(),
                None => {
                    debug!("Could not find route");
                    return HttpResponse::build(StatusCode::NOT_FOUND)
                        .insert_header(("Server", get_server_header(data.proxy.as_ref())))
                        .finish();
                },
            }
        },
    };

    // Make the request to the upstream server
    let reqwest_response = make_request(
        req.clone(),
        build_request_path(path, &route).as_ref(),
        body.clone(),
        &route.upstream,
        host,
    ).await;

    // Convert the reqwest response to an Actix response
    reqwest_response_to_actix(reqwest_response, data.proxy.as_ref()).await
}

/// Build the path that should be used in the upstream request
/// according to the settings specified in the [Route]
fn build_request_path<'a>(orig_path: &'a str, route: &Route) -> Cow<'a, str> {
    if let (Some(path_prefix), Some(strip_prefix_path)) = (&route.path_prefix, route.strip_path_prefix) {
        if strip_prefix_path {
            return Cow::Owned(orig_path.replace(path_prefix.as_str(), ""));
        }
    }

    Cow::Borrowed(orig_path)
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

fn get_server_header(proxy_config: Option<&ProxyConfig>) -> String {
    proxy_config
        .map(|x| x.error_server_header.clone())
        .flatten()
        .unwrap_or(String::default())
}

/// Turn a Reqwest response into an Actix response
async fn reqwest_response_to_actix(response: reqwest::Result<Response>, proxy_config: Option<&ProxyConfig>) -> HttpResponse {
    let response = match response {
        Ok(x) => x,
        Err(e) => return HttpResponse::build(StatusCode::BAD_GATEWAY)
                .insert_header(("Server", get_server_header(proxy_config)))
                .body(e.to_string()),
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
    path: &str,
    body: Vec<u8>,
    upstream: &str,
    original_host: &str,
) -> reqwest::Result<Response> {
    let client = Client::new();

    let request_url = if req.query_string().is_empty() {
       format!("{upstream}{path}")
    } else {
        format!("{upstream}{path}?{}", req.query_string())
    };

    let mut req_builder = client.request(
        req.method().clone(),
        &request_url,
    );

    // Some applications don't like multiple headers,
    // so we'll combine it.
    let mut header_map: HashMap<&HeaderName, Vec<&HeaderValue>> = HashMap::with_capacity(req.headers().len_keys());

    for (k, v) in req.headers() {
        header_map.entry(k)
            .and_modify(|values| values.push(v))
            .or_insert(vec![v]);
    }

    let processed_headers = header_map.into_iter()
        .map(|(k, v)| {
            let v_string = v.into_iter()
                .map(|x| x.to_str())
                .filter_map(|x| x.ok())
                .map(|x| x.to_string())
                .collect::<Vec<_>>();
            (k, v_string.join("; "))
        })
        .collect::<HashMap<_, _>>();

    for (name, value) in processed_headers {
        req_builder = req_builder.header(name, &value);
    }

    req_builder = req_builder.header("Host", original_host);

    let conninfo = req.connection_info();
    let x_forwarded_for = req.headers().get("x-forwarded-for")
        .map(|x| x.to_str().map(|x| Some(x)).unwrap_or(None))
        .flatten()
        .map(|x| if !x.is_empty() {
            format!("{x} {}", conninfo.realip_remote_addr().unwrap_or(""))
        } else { x.to_string() })
        .unwrap_or(conninfo.realip_remote_addr().unwrap_or("").to_string());

    req_builder = req_builder
        .header("X-Real-IP", conninfo.realip_remote_addr().unwrap_or(""))
        .header("X-Forwarded-For", &x_forwarded_for)
        .header("X-Forwarded-Proto", conninfo.scheme())
        .body(body);

    debug!("Sending request to {request_url}");
    req_builder.send().await
}
