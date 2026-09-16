// `asset!` in const position trips a nightly `volatile_composites` false
// positive, and `#[component]` generates an empty Props enum for prop-less
// components; neither can be addressed in user code.
#![allow(clippy::volatile_composites, clippy::empty_enums)]

use crate::Route;
use dioxus::prelude::*;

const BLOG_CSS: Asset = asset!("/assets/blog.css");

#[component]
pub fn Blog(id: i32) -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: BLOG_CSS}

        div {
            id: "blog",

            // Content
            h1 { "This is blog #{id}!" }
            p { "In blog #{id}, we show how the Dioxus router works and how URL parameters can be passed as props to our route components." }

            // Navigation links
            Link {
                to: Route::Blog { id: id - 1 },
                "Previous"
            }
            span { " <---> " }
            Link {
                to: Route::Blog { id: id + 1 },
                "Next"
            }
        }
    }
}
