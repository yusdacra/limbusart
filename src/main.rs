use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use dashmap::DashMap;
use data::{Art, ArtKind, Data};
use error::AppResult;
use http::Uri;
use maud::PreEscaped;
use std::{
    ops::Deref,
    sync::{Arc, Mutex},
};

mod data;
mod error;

#[tokio::main]
async fn main() {
    let arts_file_path = get_conf("ARTS_PATH");
    let arts = std::fs::read_to_string(&arts_file_path).unwrap();
    let state = AppState::new(Data::parse(&arts).unwrap());

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
        image_link.to_string()
    } else {
        let image_link = match art.kind {
            ArtKind::Twitter => fetch_twitter_image_link(&state.http, &art.url).await?,
            ArtKind::Safebooru => fetch_safebooru_image_link(&state.http, &art.url).await?,
        };
        state
            .direct_links
            .insert(art.url.clone(), image_link.clone());
        image_link
    };

    let page = render_page(&art, &image_link);
    Ok(page.into_response())
}

const BODY_STYLE: &str =
"margin: 0px; background: #0e0e0e; height: 100vh; width: 100vw; display: flex; font-family: \"PT Mono\", monospace; font-weight: 400; font-style: normal; font-optical-sizing: auto;";
const IMG_STYLE: &str = "display: block; margin: auto; max-height: 100vh; max-width: 100vw;";
const ABOUT_STYLE: &str = "position: absolute; bottom: 0; font-size: 0.75em; color: #ffffff; background-color: #0e0e0eaa;";

fn get_page_head_common() -> PreEscaped<String> {
    let title = get_conf("SITE_TITLE");
    let embed_title = get_conf("EMBED_TITLE");
    let embed_content = get_conf("EMBED_DESC");
    let embed_color = get_conf("EMBED_COLOR");

    maud::html! {
        meta charset="utf8";
        meta property="og:title" content=(embed_title);
        meta property="og:description" content=(embed_content);
        meta name="theme-color" content=(embed_color);
        link rel="preconnect" href="https://fonts.googleapis.com";
        link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
        link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=PT+Mono&display=swap";
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

fn render_page(art: &Art, image_link: &str) -> Html<String> {
    let content = maud::html! {
        (maud::DOCTYPE)
        head {
            (get_page_head_common())
        }
        body style=(BODY_STYLE) {
            img style=(IMG_STYLE) src=(image_link);
            a style=(format!("{ABOUT_STYLE} left: 0;")) href=(art.url) target="_blank" {
                "source: " (art.url)
            }
            (get_page_contact())
        }
    };
    Html(content.into_string())
}

async fn fetch_safebooru_image_link(http: &reqwest::Client, url: &Uri) -> AppResult<String> {
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

    let image_filename = data[0].get("image").unwrap().as_str().unwrap();
    let image_directory = data[0].get("directory").unwrap().as_str().unwrap();

    Ok(format!(
        "http://safebooru.org/images/{image_directory}/{image_filename}"
    ))
}

async fn fetch_twitter_image_link(http: &reqwest::Client, url: &Uri) -> AppResult<String> {
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
    Ok(format!("{link}?format=webp"))
}

fn get_conf(name: &str) -> String {
    std::env::var(name).unwrap()
}

struct InternalAppState {
    // cached direct links to images
    direct_links: DashMap<Uri, String>,
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
                    .user_agent(format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")))
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
