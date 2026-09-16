use dioxus::prelude::*;
use dioxus_primitives::progress::{self, ProgressProps};

#[css_module("/src/components/progress/style.css", CssModuleAssetOptions::new().with_preload(true))]
struct Styles;

#[component]
pub fn Progress(props: ProgressProps) -> Element {
    rsx! {
        progress::Progress {
            class: Styles::dx_progress,
            value: props.value,
            max: props.max,
            attributes: props.attributes,
            progress::ProgressIndicator { class: Styles::dx_progress_indicator }
        }
    }
}
