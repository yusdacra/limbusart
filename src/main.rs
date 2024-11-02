use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use dashmap::DashMap;
use data::{Art, ArtKind, Data, FetchedLink};
use error::{AppError, AppResult};
use futures_util::{future::BoxFuture, FutureExt};
use http::Uri;
use maud::PreEscaped;
use std::{
    ops::Deref,
    str::FromStr,
    sync::{Arc, Mutex},
};

mod data;
mod error;

#[tokio::main]
async fn main() {
    let arts_file_path = get_conf("ARTS_PATH").unwrap_or_else(|| "./utils/arts.txt".to_string());
    let arts = std::fs::read_to_string(&arts_file_path).unwrap();
    let state = AppState::new(Data::parse(&arts).unwrap());

    #[cfg(not(windows))]
    std::thread::spawn({
        use signal_hook::{consts::SIGUSR2, iterator::Signals};

        let state = state.clone();
        move || {
            let mut signals = Signals::new(&[SIGUSR2]).unwrap();
            for _ in signals.forever() {
                let data = std::fs::read_to_string(&arts_file_path).unwrap();
                state.data.lock().unwrap().reload(&data).unwrap();
            }
        }
    });

    let app = Router::new().route("/", get(show_art)).with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn show_art(
    headers: axum::http::HeaderMap,
    state: State<AppState>,
) -> AppResult<axum::response::Response> {
    let ua = headers
        .get(http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<unknown agent>");
    let realip = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<unknown ip>");

    println!("serving user {ua} from {realip}");

    let art = state.data.lock().unwrap().pick_random_art().clone();
    let image_link = if let Some(image_link) = state.direct_links.get(&art.url) {
        image_link.clone()
    } else {
        let image_link_fn = match art.kind {
            ArtKind::Twitter => fetch_twitter_image_link,
            ArtKind::Safebooru => fetch_safebooru_image_link,
        };
        let image_link = (image_link_fn)(&state.http, &art.url).await?;
        state
            .direct_links
            .insert(art.url.clone(), image_link.clone());
        image_link
    };

    let page = render_page(&art, &image_link);
    Ok(page.into_response())
}

const BODY_STYLE: &str =
"color: #ffffff; margin: 0px; background: #0e0e0e; height: 100vh; width: 100vw; display: flex; font-family: \"PT Mono\", monospace; font-weight: 400; font-style: normal; font-optical-sizing: auto;";
const ABOUT_STYLE: &str = "font-size: 1vmax; color: #ffffff;";

fn get_page_head_common() -> PreEscaped<String> {
    let title = get_conf("SITE_TITLE").unwrap_or_else(|| "random project moon art".to_string());
    let embed_title =
        get_conf("EMBED_TITLE").unwrap_or_else(|| "random project moon art".to_string());
    let embed_content =
        get_conf("EMBED_DESC").unwrap_or_else(|| "random project moon art".to_string());
    let embed_color = get_conf("EMBED_COLOR").unwrap_or_else(|| "#ffffff".to_string());

    maud::html! {
        meta charset="utf8";
        meta property="og:title" content=(embed_title);
        meta property="og:description" content=(embed_content);
        meta name="theme-color" content=(embed_color);
        link rel="preconnect" href="https://fonts.googleapis.com";
        link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
        link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=PT+Mono&display=swap";
        link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/@chgibb/css-spinners@2.2.1/css/spinners.min.css";
        title { (title) }
    }
}

fn get_page_contact() -> PreEscaped<String> {
    maud::html! {
        a style=(format!("{ABOUT_STYLE} right: 0;")) href="https://gaze.systems" target="_blank" {
            "website made by dusk"
            br;
            "report problems / feedback @ yusdacra on Discord"
        }
    }
}

fn render_page(art: &Art, image_link: &FetchedLink) -> Html<String> {
    let art_url = image_link.new_source.as_ref().unwrap_or(&art.url);
    let content = maud::html! {
        (maud::DOCTYPE)
        head {
            (get_page_head_common())
        }
        body style=(BODY_STYLE) {
            div style="display: block; margin: auto; max-height: 98vh; max-width: 98vw;" {
                div class="throbber-loader" style="position: absolute; top: 50%; left: 50%; z-index: -1;" {}
                img style="max-height: 98vh; max-width: 98vw;" src=(image_link.image_url);
            }
            div style="position: absolute; bottom: 0; display: flex; flex-direction: column; gap: 2vh; background-color: #0e0e0eaa;" {
                a style=(format!("{ABOUT_STYLE} left: 0;")) href=(art_url) target="_blank" {
                    "source: " (art_url)
                }
                (get_page_contact())
            }
        }
    };
    Html(content.into_string())
}

fn fetch_safebooru_image_link<'a>(
    http: &'a reqwest::Client,
    url: &'a Uri,
) -> BoxFuture<'a, AppResult<FetchedLink>> {
    _fetch_safebooru_image_link(http, url).boxed()
}

fn fetch_twitter_image_link<'a>(
    http: &'a reqwest::Client,
    url: &'a Uri,
) -> BoxFuture<'a, AppResult<FetchedLink>> {
    _fetch_twitter_image_link(http, url).boxed()
}

async fn _fetch_safebooru_image_link(http: &reqwest::Client, url: &Uri) -> AppResult<FetchedLink> {
    let mut id = String::new();
    for (name, value) in form_urlencoded::parse(url.query().unwrap().as_bytes()) {
        if name == "id" {
            id = value.into_owned();
        }
    }
    if id.is_empty() {
        return Err("no id?".into());
    }

    let url = format!("https://safebooru.org/index.php?page=dapi&s=post&q=index&json=1&id={id}");
    type Data = Vec<serde_json::Map<String, serde_json::Value>>;
    let try_request = || {
        let url = url.clone();
        let http = http.clone();
        async move {
            println!("[safebooru] trying to fetch url: {url}");
            let req = http.get(url).build()?;
            let resp = http.execute(req).await?.error_for_status()?;
            let data = resp.json::<Data>().await?;
            AppResult::Ok(data)
        }
    };

    let mut attempts: usize = 0;
    let (data, _) = futures_retry::FutureRetry::new(try_request, |e| {
        if attempts > 4 {
            futures_retry::RetryPolicy::<error::AppError>::ForwardError(e)
        } else {
            attempts += 1;
            println!("[safebooru] retrying url fetch (attempt {attempts}): {url}");
            futures_retry::RetryPolicy::<error::AppError>::Repeat
        }
    })
    .await
    .map_err(|(e, _)| e)?;

    let source_url = data[0]
        .get("source")
        .and_then(|src| Uri::from_str(src.as_str()?).ok())
        .map(|src| {
            if src.host() == Some("i.pximg.net") {
                let post_id = src
                    .path()
                    .split('/')
                    .last()
                    .unwrap()
                    .split("_")
                    .next()
                    .unwrap();
                return Uri::builder()
                    .scheme("https")
                    .authority("pixiv.net")
                    .path_and_query(format!("/en/artworks/{post_id}"))
                    .build()
                    .unwrap();
            } else {
                src
            }
        });

    if source_url.as_ref().map_or(false, |src| {
        src.host().unwrap().contains("twitter.com") || src.host().unwrap().contains("x.com")
    }) {
        let url = source_url.clone().unwrap();
        println!("[safebooru] source was twitter, will try to fetch image from there instead");
        if let Ok(mut fetched) = _fetch_twitter_image_link(http, &url).await {
            println!("[safebooru] fetched image from twitter");
            fetched.new_source = Some(url);
            return Ok(fetched);
        }
    }

    let sample_url = data[0]
        .get("sample_url")
        .ok_or("safebooru did not return sample url")?
        .as_str()
        .ok_or("safebooru sample url wasnt a string")?;
    let sample_url = Uri::from_str(sample_url)
        .map_err(|err| AppError::from(format!("safebooru sample url was not valid: {err}")))?;

    let fsample_url = format!(
        "{}://{}{}",
        sample_url.scheme_str().unwrap(),
        sample_url.host().unwrap(),
        sample_url.path()
    );
    let ssample_url = format!(
        "{}://{}/{}",
        sample_url.scheme_str().unwrap(),
        sample_url.host().unwrap(),
        sample_url.path()
    );

    let fsample_resp = http
        .execute(http.get(&fsample_url).build()?)
        .await
        .and_then(|resp| resp.error_for_status());
    let ssample_resp = http
        .execute(http.get(&ssample_url).build()?)
        .await
        .and_then(|resp| resp.error_for_status());

    let sample_url = fsample_resp
        .is_ok()
        .then(|| fsample_url)
        .or_else(|| ssample_resp.is_ok().then(|| ssample_url))
        .unwrap_or_else(|| sample_url.to_string());

    Ok(FetchedLink {
        image_url: sample_url,
        new_source: source_url,
    })
}

async fn _fetch_twitter_image_link(http: &reqwest::Client, url: &Uri) -> AppResult<FetchedLink> {
    let fxurl = Uri::builder()
        .scheme("https")
        .authority("d.fxtwitter.com")
        .path_and_query(url.path_and_query().unwrap().clone())
        .build()?
        .to_string();
    println!("[fxtwitter] trying to fetch url: {fxurl}");
    let req = http.get(&fxurl).build()?;
    let resp = http.execute(req).await?.error_for_status()?;
    let link = resp
        .headers()
        .get(http::header::LOCATION)
        .ok_or_else(|| format!("twitter link {fxurl} did not return an image location"))?
        .to_str()?;
    // use webp format for direct twitter links since webp is cheaper
    Ok(FetchedLink {
        image_url: format!("{link}?format=webp"),
        new_source: None,
    })
}

fn get_conf(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

struct InternalAppState {
    // cached direct links to images
    direct_links: DashMap<Uri, FetchedLink>,
    data: Mutex<Data>,
    http: reqwest::Client,
}

#[derive(Clone)]
struct AppState {
    internal: Arc<InternalAppState>,
}

impl AppState {
    fn new(data: Data) -> Self {
        Self {
            internal: Arc::new(InternalAppState {
                data: Mutex::new(data),
                direct_links: Default::default(),
                http: reqwest::ClientBuilder::new()
                    .redirect(reqwest::redirect::Policy::none())
                    .user_agent(format!(
                        "{}/{}",
                        env!("CARGO_PKG_NAME"),
                        env!("CARGO_PKG_VERSION")
                    ))
                    .build()
                    .unwrap(),
            }),
        }
    }
}

impl Deref for AppState {
    type Target = InternalAppState;

    fn deref(&self) -> &Self::Target {
        &self.internal
    }
}
