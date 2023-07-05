use crate::errors::{Error, MeilisearchCommunicationError, MeilisearchError};
use lazy_static::lazy_static;
use log::{error, trace, warn};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::from_str;

lazy_static! {
    pub(crate) static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

#[derive(Debug)]
pub(crate) enum Method<Q, B> {
    Get { query: Q },
    Post { query: Q, body: B },
    Patch { query: Q, body: B },
    Put { query: Q, body: B },
    Delete { query: Q },
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn request<
    Query: Serialize,
    Body: Serialize,
    Output: DeserializeOwned + 'static,
>(
    url: &str,
    apikey: Option<&str>,
    method: Method<Query, Body>,
    expected_status_code: u16,
) -> Result<Output, Error> {
    use http::header;

    let (http_method, query, body) = match method {
        Method::Get { query } => (http::Method::GET, Some(query), None),
        Method::Post { query, body } => (http::Method::POST, Some(query), Some(body)),
        Method::Patch { query, body } => (http::Method::PATCH, Some(query), Some(body)),
        Method::Put { query, body } => (http::Method::PUT, Some(query), Some(body)),
        Method::Delete { query } => (http::Method::DELETE, Some(query), None),
    };

    let builder = CLIENT
        .request(http_method, url)
        .header(header::USER_AGENT, qualified_version());
    let mut builder = match apikey {
        Some(apikey) => builder.header(header::AUTHORIZATION, format!("Bearer {apikey}")),
        None => builder,
    };

    if let Some(query) = query {
        builder = builder.query(&query);
    }

    if let Some(body) = body {
        builder = builder.json(&body);
    }

    let response = builder.send().await?;
    let status = response.status().as_u16();

    let mut body = response.text().await?;

    if body.is_empty() {
        body = "null".to_string();
    }

    parse_response(status, expected_status_code, &body, url.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn stream_request<
    'a,
    Query: Serialize,
    Body: futures_io::AsyncRead + Send + Sync + 'static,
    Output: DeserializeOwned + 'static,
>(
    url: &str,
    apikey: Option<&str>,
    method: Method<Query, Body>,
    content_type: &str,
    expected_status_code: u16,
) -> Result<Output, Error> {
    use http::header;
    use tokio_util::compat::FuturesAsyncReadCompatExt;
    use tokio_util::io::ReaderStream;

    let (http_method, query, body) = match method {
        Method::Get { query } => (http::Method::GET, Some(query), None),
        Method::Post { query, body } => (http::Method::POST, Some(query), Some(body)),
        Method::Patch { query, body } => (http::Method::PATCH, Some(query), Some(body)),
        Method::Put { query, body } => (http::Method::PUT, Some(query), Some(body)),
        Method::Delete { query } => (http::Method::DELETE, Some(query), None),
    };

    let builder = CLIENT
        .request(http_method, url)
        .header(header::USER_AGENT, qualified_version())
        .header(header::CONTENT_TYPE, content_type);
    let mut builder = match apikey {
        Some(apikey) => builder.header(header::AUTHORIZATION, format!("Bearer {apikey}")),
        None => builder,
    };

    if let Some(query) = query {
        builder = builder.query(&query);
    }

    if let Some(body) = body {
        builder = builder.body(reqwest::Body::wrap_stream(ReaderStream::new(body.compat())));
    }

    let response = builder.send().await?;
    let status = response.status().as_u16();

    let mut body = response.text().await?;

    if body.is_empty() {
        body = "null".to_string();
    }

    parse_response(status, expected_status_code, &body, url.to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn add_query_parameters<Query: Serialize>(
    mut url: String,
    query: &Query,
) -> Result<String, Error> {
    let query = yaup::to_string(query)?;

    if !query.is_empty() {
        url = format!("{}?{}", url, query);
    };
    return Ok(url);
}
#[cfg(target_arch = "wasm32")]
pub(crate) async fn request<
    Query: Serialize,
    Body: Serialize,
    Output: DeserializeOwned + 'static,
>(
    url: &str,
    apikey: Option<&str>,
    method: Method<Query, Body>,
    expected_status_code: u16,
) -> Result<Output, Error> {
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, RequestInit, Response};

    const CONTENT_TYPE: &str = "Content-Type";
    const JSON: &str = "application/json";

    // The 2 following unwraps should not be able to fail
    let mut mut_url = url.clone().to_string();
    let headers = Headers::new().unwrap();
    if let Some(apikey) = apikey {
        headers
            .append("Authorization", format!("Bearer {}", apikey).as_str())
            .unwrap();
    }
    headers
        .append("X-Meilisearch-Client", qualified_version().as_str())
        .unwrap();

    let mut request: RequestInit = RequestInit::new();
    request.headers(&headers);

    match &method {
        Method::Get { query } => {
            mut_url = add_query_parameters(mut_url, &query)?;

            request.method("GET");
        }
        Method::Delete { query } => {
            mut_url = add_query_parameters(mut_url, &query)?;
            request.method("DELETE");
        }
        Method::Patch { query, body } => {
            mut_url = add_query_parameters(mut_url, &query)?;
            request.method("PATCH");
            headers.append(CONTENT_TYPE, JSON).unwrap();
            request.body(Some(&JsValue::from_str(&to_string(body).unwrap())));
        }
        Method::Post { query, body } => {
            mut_url = add_query_parameters(mut_url, &query)?;
            request.method("POST");
            headers.append(CONTENT_TYPE, JSON).unwrap();
            request.body(Some(&JsValue::from_str(&to_string(body).unwrap())));
        }
        Method::Put { query, body } => {
            mut_url = add_query_parameters(mut_url, &query)?;
            request.method("PUT");
            headers.append(CONTENT_TYPE, JSON).unwrap();
            request.body(Some(&JsValue::from_str(&to_string(body).unwrap())));
        }
    }

    let window = web_sys::window().unwrap(); // TODO remove this unwrap
    let response =
        match JsFuture::from(window.fetch_with_str_and_init(mut_url.as_str(), &request)).await {
            Ok(response) => Response::from(response),
            Err(e) => {
                error!("Network error: {:?}", e);
                return Err(Error::UnreachableServer);
            }
        };
    let status = response.status() as u16;
    let text = match response.text() {
        Ok(text) => match JsFuture::from(text).await {
            Ok(text) => text,
            Err(e) => {
                error!("Invalid response: {:?}", e);
                return Err(Error::HttpError("Invalid response".to_string()));
            }
        },
        Err(e) => {
            error!("Invalid response: {:?}", e);
            return Err(Error::HttpError("Invalid response".to_string()));
        }
    };

    if let Some(t) = text.as_string() {
        if t.is_empty() {
            parse_response(status, expected_status_code, "null", url.to_string())
        } else {
            parse_response(status, expected_status_code, &t, url.to_string())
        }
    } else {
        error!("Invalid response");
        Err(Error::HttpError("Invalid utf8".to_string()))
    }
}

fn parse_response<Output: DeserializeOwned>(
    status_code: u16,
    expected_status_code: u16,
    body: &str,
    url: String,
) -> Result<Output, Error> {
    if status_code == expected_status_code {
        match from_str::<Output>(body) {
            Ok(output) => {
                trace!("Request succeed");
                return Ok(output);
            }
            Err(e) => {
                error!("Request succeeded but failed to parse response");
                return Err(Error::ParseError(e));
            }
        };
    }

    warn!(
        "Expected response code {}, got {}",
        expected_status_code, status_code
    );

    match from_str::<MeilisearchError>(body) {
        Ok(e) => Err(Error::from(e)),
        Err(e) => {
            if status_code >= 400 {
                return Err(Error::MeilisearchCommunication(
                    MeilisearchCommunicationError {
                        status_code,
                        message: None,
                        url,
                    },
                ));
            }
            Err(Error::ParseError(e))
        }
    }
}

pub fn qualified_version() -> String {
    const VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");

    format!("Meilisearch Rust (v{})", VERSION.unwrap_or("unknown"))
}
