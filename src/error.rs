use std::fmt::Display;

use axum::response::{Html, IntoResponse};
use http::StatusCode;

type BoxedError = Box<dyn std::error::Error>;

pub(crate) type AppResult<T> = Result<T, AppError>;

#[derive(Debug)]
pub(crate) struct AppError {
    internal: BoxedError,
    status: Option<StatusCode>,
}

impl AppError {
    pub(crate) fn status(mut self, code: StatusCode) -> Self {
        self.status = Some(code);
        self
    }
}

impl<E> From<E> for AppError
where
    E: Into<BoxedError>,
{
    fn from(err: E) -> Self {
        Self {
            internal: err.into(),
            status: None,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let pre_escaped = maud::html! {
            (maud::DOCTYPE)
            head {
                (crate::get_page_head_common())
            }
            body style=(crate::BODY_STYLE) {
                p style=("display: block; margin: auto; font-size: 1.3em;") {
                    "Something went wrong: "
                    br;
                    (self.internal.to_string());
                }
                (crate::get_page_contact())
            }
        };
        let mut resp = Html(pre_escaped.into_string()).into_response();

        *resp.status_mut() = self.status.unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        resp
    }
}

impl Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.internal.fmt(f)
    }
}
