use std::borrow::Cow;
use std::collections::HashMap;
use crate::Config;
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web::http::header::{HeaderName, HeaderValue};
use anyhow::Result;
use futures_util::StreamExt;
use reqwest::{Client, Response, StatusCode, Version};
use tracing::{warn, instrument, debug, trace};
use crate::config::{ProxyConfig, Route};

#[instrument(skip(data, req, payload))]
pub async fn proxy(
    data: web::Data<Config>,
    req: HttpRequest,
    payload: web::Payload
) -> HttpResponse {
    let path = req.path();
    let host = match get_request_host(&req) {
        Some(x) => x,
        None => return HttpResponse::new(StatusCode::BAD_GATEWAY)
    };

    let body = match extract_body(payload).await {
        Ok(x) => x,
        Err(e) => {
            warn!("Failed to extract request body: {e}");
            return HttpResponse::new(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    let route = match choose_route(host, path, data.routes.iter().collect::<Vec<_>>()) {
        Some(x) => x,
        None => {
            debug!("Could not find route");
            return HttpResponse::build(StatusCode::NOT_FOUND)
                .insert_header(("Server", get_server_header(data.proxy.as_ref())))
                .finish();
        }
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
    reqwest_response_to_actix(reqwest_response, data.proxy.as_ref(), &route).await
}

fn choose_route<'a>(host: &str, path: &str, routes: Vec<&'a Route>) -> Option<&'a Route> {
    let mut route_has_host_and_path = Vec::new();
    let mut route_has_host = Vec::new();
    let mut route_has_path = Vec::new();
    let mut default_routes = Vec::new();

    for route in routes {
        if let (Some(route_host), Some(route_path)) = (&route.host, &route.path_prefix) {
            if route_host.eq(host) && path.starts_with(route_path) {
                route_has_host_and_path.push(route);
            }
        }

        if let Some(route_host) = &route.host {
            if route_host.eq(host) {
                route_has_host.push(route);
            }
        }

        if let Some(route_path) = &route.path_prefix {
            if path.starts_with(route_path) {
                route_has_path.push(route);
            }
        }

        if let Some(default) = route.default {
            if default {
                default_routes.push(route);
            }
        }
    }

    if let Some(route) = route_has_host_and_path.first() {
        trace!("Host and path route chosen");
        return Some(route);
    }

    else if let Some(route) = route_has_host.first() {
        trace!("Host route chosen");
        return Some(route);
    }

    if let Some(route) = route_has_path.first() {
        trace!("Path route chosen");
        return Some(route);
    }

    if let Some(route) = default_routes.first() {
        trace!("Default route chosen");
        return Some(route);
    }

    None
}

fn get_request_host(req: &HttpRequest) -> Option<&str> {
    let host = match req.headers().get("host") {
        Some(h) => h.to_str().ok(),
        None => req.uri().host()
    };

    trace!("Got Host {host:?}");
    host
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
async fn reqwest_response_to_actix(response: reqwest::Result<Response>, proxy_config: Option<&ProxyConfig>, route: &Route) -> HttpResponse {
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

    if let Some(response_headers) = &route.response_headers {
        for (k, v) in response_headers {
            builder.insert_header((&**k, &**v));
        }
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
    )
        .version(Version::HTTP_11);

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
        if name.as_str().to_lowercase().eq("host") {
            continue;
        }

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
        .header("X-Forwarded-Host", original_host)
        .body(body);

    debug!("Sending request to {request_url}");
    req_builder.send().await
}
