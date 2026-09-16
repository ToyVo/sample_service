use dioxus::prelude::*;
use dioxus_icons::lucide::ChevronDown;
use dioxus_primitives::collapsible::{
    self, CollapsibleContentProps, CollapsibleProps, CollapsibleTriggerProps,
};
use dioxus_primitives::{dioxus_attributes::attributes, merge_attributes};

#[css_module("/src/components/collapsible/style.css", CssModuleAssetOptions::new().with_preload(true))]
struct Styles;

/// Independent show/hide section.
///
/// Unlike `accordion`, each `Collapsible` owns its own `Memo<bool>` open
/// state (`use_controlled`), so toggling one never re-renders siblings.
/// Closed content is unmounted immediately — no `document::eval`
/// `getAnimations()` round-trip like `accordion`'s `use_animated_open`.
#[component]
pub fn Collapsible(props: CollapsibleProps) -> Element {
    let base = attributes!(div {
        class: Styles::dx_collapsible,
    });
    let merged = merge_attributes(vec![base, props.attributes]);
    rsx! {
        collapsible::Collapsible {
            keep_mounted: props.keep_mounted,
            default_open: props.default_open,
            disabled: props.disabled,
            open: props.open,
            on_open_change: props.on_open_change,
            r#as: props.r#as,
            attributes: merged,
            {props.children}
        }
    }
}

/// Button that toggles the sibling content. Chevron `rotate` transition is
/// compositor-accelerated (no layout).
#[component]
pub fn CollapsibleTrigger(props: CollapsibleTriggerProps) -> Element {
    let base = attributes!(button {
        class: Styles::dx_collapsible_trigger,
    });
    let merged = merge_attributes(vec![base, props.attributes]);
    let show_icon = props.r#as.is_none();
    rsx! {
        collapsible::CollapsibleTrigger {
            r#as: props.r#as,
            attributes: merged,
            {props.children}
            if show_icon {
                ChevronDown {
                    class: Styles::dx_collapsible_expand_icon,
                    size: "20px",
                    stroke: "var(--secondary-color-4)",
                }
            }
        }
    }
}

/// Content mounted only when open (or `keep_mounted`). Enter animation uses
/// `opacity`/`transform` only — both compositor props, no `height` /
/// `grid-template-rows` reflow. Exit is instant unmount.
#[component]
pub fn CollapsibleContent(props: CollapsibleContentProps) -> Element {
    let base = attributes!(div {
        class: Styles::dx_collapsible_content,
    });
    let merged = merge_attributes(vec![base, props.attributes]);
    rsx! {
        collapsible::CollapsibleContent {
            id: props.id,
            attributes: merged,
            {props.children}
        }
    }
}

/// Stack of sections with dividers. Scopes layout invalidation via
/// `contain: layout style` on children (see stylesheet).
#[component]
pub fn CollapsibleList(
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    rsx! {
        div {
            class: Styles::dx_collapsible_list,
            ..attributes,
            {children}
        }
    }
}
