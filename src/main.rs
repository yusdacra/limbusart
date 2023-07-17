use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use dashmap::DashMap;
use error::AppError;
use http::Uri;
use std::{
    collections::HashMap,
    ops::Deref,
    str::FromStr,
    sync::{Arc, Mutex},
};

mod error;

type AppResult<T> = Result<T, AppError>;

#[derive(Clone)]
enum ArtKind {
    Twitter,
}

impl FromStr for ArtKind {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "twitter.com" => Ok(Self::Twitter),
            _ => Err("not support website".into()),
        }
    }
}

#[derive(Clone)]
struct Art {
    url: Uri,
    kind: ArtKind,
}

impl FromStr for Art {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: Uri = s.parse()?;
        let kind: ArtKind = url.authority().unwrap().host().parse()?;

        Ok(Self { url, kind })
    }
}

struct Data {
    // actual arts
    art: Vec<Art>,
    art_indices: HashMap<Uri, usize>,
}

impl Data {
    fn parse(data: &str) -> AppResult<Self> {
        let mut this = Self {
            art: Default::default(),
            art_indices: Default::default(),
        };

        for entry in data.lines() {
            let art: Art = entry.parse()?;
            this.art_indices.insert(art.url.clone(), this.art.len());
            this.art.push(art);
        }

        Ok(this)
    }

    fn pick_random_art(&self) -> &Art {
        let no = fastrand::usize(0..self.art.len());
        &self.art[no]
    }

    fn reload(&mut self, data: &str) -> AppResult<()> {
        for entry in data.lines() {
            let art: Art = entry.parse()?;
            if !self.art_indices.contains_key(&art.url) {
                self.art_indices.insert(art.url.clone(), self.art.len());
                self.art.push(art);
            }
        }
        Ok(())
    }
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

const ARTS_PATH: &str = "arts.txt";

#[tokio::main]
async fn main() {
    let arts = std::fs::read_to_string(ARTS_PATH).unwrap();
    let state = AppState::new(Data::parse(&arts).unwrap());

    std::thread::spawn({
        use signal_hook::{consts::SIGUSR2, iterator::Signals};

        let state = state.clone();
        move || {
            let mut signals = Signals::new(&[SIGUSR2]).unwrap();
            for _ in signals.forever() {
                let data = std::fs::read_to_string(ARTS_PATH).unwrap();
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
    state: State<AppState>,
    headers: http::HeaderMap,
) -> AppResult<axum::response::Response> {
    let art = state.data.lock().unwrap().pick_random_art().clone();
    let image_link = if let Some(image_link) = state.direct_links.get(&art.url) {
        image_link.to_string()
    } else {
        let image_link = match art.kind {
            ArtKind::Twitter => fetch_twitter_image_link(&state.http, &art.url).await?,
        };
        state
            .direct_links
            .insert(art.url.clone(), image_link.clone());
        image_link
    };

    if let Some(agent) = headers
        .get(http::header::USER_AGENT)
        .and_then(|h| h.to_str().ok())
    {
        if agent.contains("Discordbot") {
            let request = state.http.get(&image_link).build()?;
            let resp = state.http.execute(request).await?.error_for_status()?;
            let headers = resp.headers().clone();
            let downloaded = resp.bytes().await?;
            let mut response = axum::response::Response::new(downloaded.into());
            *response.headers_mut() = headers;
            response.headers_mut().remove(http::header::CACHE_CONTROL);
            response.headers_mut().remove(http::header::CACHE_STATUS);
            return Ok(response);
        }
    }

    let page = render_page(&art, &image_link);
    Ok(page.into_response())
}

fn render_page(art: &Art, image_link: &str) -> Html<String> {
    let body_style =
        "margin: 0px; background: #0e0e0e; height: 100vh; width: 100vw; display: flex;";
    let img_style = "display: block; margin: auto; max-height: 100vh; max-width: 100vw;";
    let about_style = "position: absolute; bottom: 0; font-size: 0.75em; color: #ffffff; background-color: #0e0e0eaa;";
    let content = maud::html! {
        (maud::DOCTYPE)
        head {
            meta charset="utf8";
            meta property="og:image" content=(image_link);
            title { "random limbussy art" }
        }
        body style=(body_style) {
            img style=(img_style) src=(image_link);
            a style=(format!("{about_style} left: 0;")) href=(art.url) target="_blank" {
                "source: " (art.url)
            }
            a style=(format!("{about_style} right: 0;")) href="https://gaze.systems" target="_blank" {
                "website made by dusk"
            }
        }
    };
    Html(content.into_string())
}

async fn fetch_twitter_image_link(http: &reqwest::Client, url: &Uri) -> AppResult<String> {
    let fxurl = Uri::builder()
        .scheme("https")
        .authority("d.fxtwitter.com")
        .path_and_query(url.path_and_query().unwrap().clone())
        .build()?
        .to_string();
    let req = http.get(fxurl).build()?;
    let resp = http.execute(req).await?.error_for_status()?;
    let link = resp
        .headers()
        .get(http::header::LOCATION)
        .unwrap()
        .to_str()?
        .to_owned();
    Ok(link)
}
