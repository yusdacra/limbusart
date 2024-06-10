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

async fn show_art(state: State<AppState>) -> AppResult<axum::response::Response> {
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

fn render_page(art: &Art, image_link: &str) -> Html<String> {
    let title = get_conf("SITE_TITLE");
    let embed_title = get_conf("EMBED_TITLE");
    let embed_content = get_conf("EMBED_DESC");
    let embed_color = get_conf("EMBED_COLOR");

    let body_style =
        "margin: 0px; background: #0e0e0e; height: 100vh; width: 100vw; display: flex;";
    let img_style = "display: block; margin: auto; max-height: 100vh; max-width: 100vw;";
    let about_style = "position: absolute; bottom: 0; font-size: 0.75em; color: #ffffff; background-color: #0e0e0eaa;";
    let content = maud::html! {
        (maud::DOCTYPE)
        head {
            meta charset="utf8";
            meta property="og:title" content=(embed_title);
            meta property="og:description" content=(embed_content);
            meta name="theme-color" content=(embed_color);
            title { (title) }
        }
        body style=(body_style) {
            img style=(img_style) src=(image_link);
            a style=(format!("{about_style} left: 0;")) href=(art.url) target="_blank" {
                "source: " (art.url)
            }
            a style=(format!("{about_style} right: 0;")) href="https://gaze.systems" target="_blank" {
                "website made by dusk"
                br;
                "report problems / feedback @ yusdacra on Discord"
            }
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
    let req = http.get(url).build()?;
    let resp = http.execute(req).await?.error_for_status()?;
    let data: Vec<serde_json::Map<String, serde_json::Value>> = resp.json().await?;

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
