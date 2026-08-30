//! Context-usage panel domain: the signals that own the panel's open/dock/
//! drag/resize state, the pointer behaviour that mutates them, and the panel
//! component itself. `App` only constructs `ContextUsageState`, wires the
//! callbacks, and places `<ContextUsagePanel/>` in the two slots.

use super::*;
use crate::chat_render::{
    context_percent, context_usage_detail_text, context_usage_rows, context_usage_tone,
    fmt_context_limit, fmt_context_tokens, ContextUsageTone,
};

const CONTEXT_USAGE_DRAG_THRESHOLD: f64 = 8.0;

fn context_usage_event_target(ev: &web_sys::MouseEvent) -> Option<web_sys::Element> {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
}

fn context_usage_panel_el() -> Option<web_sys::Element> {
    web_sys::window()?
        .document()?
        .query_selector("[data-testid='context-usage-panel']")
        .ok()
        .flatten()
}

pub(crate) fn event_inside_selector(ev: &web_sys::MouseEvent, selector: &str) -> bool {
    context_usage_event_target(ev)
        .and_then(|element| element.closest(selector).ok().flatten())
        .is_some()
}

/// All reactive state of the context-usage panel. `RwSignal` is `Copy`, so the
/// whole struct is `Copy` and can be captured by every closure that needs it.
#[derive(Clone, Copy)]
pub(crate) struct ContextUsageState {
    pub(crate) open: RwSignal<bool>,
    pub(crate) mode: RwSignal<ContextUsageMode>,
    pub(crate) geom: RwSignal<Option<ContextUsageGeom>>,
    pub(crate) dragging: RwSignal<bool>,
    pub(crate) tracking: RwSignal<bool>,
    pub(crate) resizing: RwSignal<bool>,
    pub(crate) drag_origin: RwSignal<(f64, f64)>,
    pub(crate) grab: RwSignal<(f64, f64)>,
    pub(crate) resize_origin: RwSignal<(f64, f64)>,
    pub(crate) resize_start: RwSignal<ContextUsageGeom>,
    pub(crate) passed_threshold: RwSignal<bool>,
    pub(crate) suppress_click: RwSignal<bool>,
    pub(crate) details: RwSignal<Option<ContextUsageDetails>>,
    pub(crate) detail_open: RwSignal<Option<String>>,
}

impl ContextUsageState {
    pub(crate) fn new() -> Self {
        Self {
            open: create_rw_signal(false),
            mode: create_rw_signal(ContextUsageMode::Docked),
            geom: create_rw_signal(load_context_usage_geom()),
            dragging: create_rw_signal(false),
            tracking: create_rw_signal(false),
            resizing: create_rw_signal(false),
            drag_origin: create_rw_signal((0.0_f64, 0.0_f64)),
            grab: create_rw_signal((0.0_f64, 0.0_f64)),
            resize_origin: create_rw_signal((0.0_f64, 0.0_f64)),
            resize_start: create_rw_signal(ContextUsageGeom {
                x: 0.0,
                y: 0.0,
                w: CONTEXT_USAGE_DEFAULT_W,
                h: CONTEXT_USAGE_DEFAULT_H,
            }),
            passed_threshold: create_rw_signal(false),
            suppress_click: create_rw_signal(false),
            details: create_rw_signal(None),
            detail_open: create_rw_signal(None),
        }
    }

    pub(crate) fn header_down(self, ev: web_sys::MouseEvent) {
        if ev.button() != 0 || event_inside_selector(&ev, "button") {
            return;
        }
        let x = ev.client_x() as f64;
        let y = ev.client_y() as f64;
        self.tracking.set(true);
        self.passed_threshold.set(false);
        self.drag_origin.set((x, y));
        if let Some(panel) = context_usage_panel_el() {
            let rect = panel.get_bounding_client_rect();
            self.grab.set((x - rect.x(), y - rect.y()));
            if self.geom.get_untracked().is_none() {
                self.geom.set(Some(ContextUsageGeom {
                    x: rect.x(),
                    y: rect.y(),
                    w: rect.width(),
                    h: rect.height(),
                }));
            }
        }
    }

    pub(crate) fn header_dblclick(self, ev: web_sys::MouseEvent) {
        if event_inside_selector(&ev, "button") {
            return;
        }
        if self.mode.get_untracked() == ContextUsageMode::Floating {
            self.mode.set(ContextUsageMode::Docked);
        }
    }

    pub(crate) fn dock(self) {
        self.mode.set(ContextUsageMode::Docked);
    }

    pub(crate) fn drag_move(self, ev: web_sys::MouseEvent) {
        if !self.tracking.get() && !self.dragging.get() {
            return;
        }
        let x = ev.client_x() as f64;
        let y = ev.client_y() as f64;
        let (origin_x, origin_y) = self.drag_origin.get();
        let dx = x - origin_x;
        let dy = y - origin_y;
        if !self.passed_threshold.get() {
            if (dx * dx + dy * dy).sqrt() < CONTEXT_USAGE_DRAG_THRESHOLD {
                return;
            }
            self.passed_threshold.set(true);
            self.dragging.set(true);
            if self.mode.get_untracked() == ContextUsageMode::Docked {
                let (grab_x, grab_y) = self.grab.get_untracked();
                let (width, height) = self
                    .geom
                    .get_untracked()
                    .map(|geom| (geom.w, geom.h))
                    .or_else(|| {
                        context_usage_panel_el().map(|panel| {
                            let rect = panel.get_bounding_client_rect();
                            (rect.width(), rect.height())
                        })
                    })
                    .unwrap_or((CONTEXT_USAGE_DEFAULT_W, CONTEXT_USAGE_DEFAULT_H));
                let (viewport_w, viewport_h) = viewport_size();
                self.geom.set(Some(clamp_context_usage_geom(
                    x - grab_x,
                    y - grab_y,
                    width,
                    height,
                    viewport_w,
                    viewport_h,
                )));
                self.mode.set(ContextUsageMode::Floating);
            }
        }
        if self.mode.get_untracked() != ContextUsageMode::Floating {
            return;
        }
        let (grab_x, grab_y) = self.grab.get_untracked();
        let (width, height) = self
            .geom
            .get_untracked()
            .map(|geom| (geom.w, geom.h))
            .unwrap_or((CONTEXT_USAGE_DEFAULT_W, CONTEXT_USAGE_DEFAULT_H));
        let (viewport_w, viewport_h) = viewport_size();
        self.geom.set(Some(clamp_context_usage_geom(
            x - grab_x,
            y - grab_y,
            width,
            height,
            viewport_w,
            viewport_h,
        )));
    }

    pub(crate) fn drag_end(self) {
        if !self.tracking.get() && !self.dragging.get() {
            return;
        }
        let moved = self.passed_threshold.get();
        self.tracking.set(false);
        self.dragging.set(false);
        if moved {
            if let Some(geom) = self.geom.get() {
                save_context_usage_geom(geom);
            }
            self.suppress_click.set(true);
        }
        self.passed_threshold.set(false);
    }

    pub(crate) fn resize_begin(self, ev: web_sys::MouseEvent) {
        if ev.button() != 0 {
            return;
        }
        ev.prevent_default();
        ev.stop_propagation();
        let geom = context_usage_panel_el()
            .map(|panel| {
                let rect = panel.get_bounding_client_rect();
                ContextUsageGeom {
                    x: rect.x(),
                    y: rect.y(),
                    w: rect.width(),
                    h: rect.height(),
                }
            })
            .or_else(|| self.geom.get_untracked());
        let Some(geom) = geom else {
            return;
        };
        self.geom.set(Some(geom));
        self.resizing.set(true);
        self.resize_origin
            .set((ev.client_x() as f64, ev.client_y() as f64));
        self.resize_start.set(geom);
    }

    pub(crate) fn resize_move(self, ev: web_sys::MouseEvent) {
        if !self.resizing.get() {
            return;
        }
        let start = self.resize_start.get();
        let (origin_x, origin_y) = self.resize_origin.get();
        let (viewport_w, viewport_h) = viewport_size();
        self.geom.set(Some(clamp_context_usage_geom(
            start.x,
            start.y,
            start.w + ev.client_x() as f64 - origin_x,
            start.h + ev.client_y() as f64 - origin_y,
            viewport_w,
            viewport_h,
        )));
    }

    pub(crate) fn resize_end(self) {
        if !self.resizing.get() {
            return;
        }
        self.resizing.set(false);
        if let Some(geom) = self.geom.get() {
            save_context_usage_geom(geom);
        }
        self.suppress_click.set(true);
    }
}

#[component]
pub(crate) fn ContextUsagePanel(
    snapshot: ContextUsageSnapshot,
    floating: bool,
    locale: ReadSignal<Locale>,
    context_usage_open: RwSignal<bool>,
    context_usage_details: RwSignal<Option<ContextUsageDetails>>,
    context_usage_detail_open: RwSignal<Option<String>>,
    context_usage_geom: RwSignal<Option<ContextUsageGeom>>,
    on_header_down: Callback<web_sys::MouseEvent>,
    on_header_dblclick: Callback<web_sys::MouseEvent>,
    on_dock: Callback<()>,
    on_resize_start: Callback<web_sys::MouseEvent>,
    on_compact: Callback<()>,
    on_new_session: Callback<()>,
    compact_disabled: Signal<bool>,
) -> impl IntoView {
    let loc = locale.get();
    let pct = context_percent(snapshot.used, snapshot.max);
    let danger = context_usage_tone(snapshot.used, snapshot.max) == ContextUsageTone::Danger;
    let used = fmt_context_tokens(snapshot.used);
    let total = if snapshot.max == 0 {
        tf(loc, "context_usage.total_used", &[("used", &used)])
    } else {
        let max = fmt_context_limit(snapshot.max);
        tf(
            loc,
            if snapshot.estimated {
                "context_usage.total_estimated"
            } else {
                "context_usage.total_exact"
            },
            &[("used", &used), ("max", &max)],
        )
    };
    let rows = context_usage_rows(&snapshot, loc);
    let segments = rows.clone();
    let denominator = snapshot.max.max(snapshot.used).max(1);
    let mode = if floating { "floating" } else { "docked" };
    view! {
        <section id="context-usage-panel" class="context-usage-panel"
            class:is-docked=!floating
            class:is-floating=floating
            data-testid="context-usage-panel"
            data-mode=mode
            role="dialog"
            aria-labelledby="context-usage-title"
            style=move || {
                if !floating {
                    String::new()
                } else if let Some(geom) = context_usage_geom.get() {
                    format!(
                        "left:{}px;top:{}px;width:{}px;height:{}px;--context-usage-h:{}px",
                        geom.x, geom.y, geom.w, geom.h, geom.h
                    )
                } else {
                    String::new()
                }
            }>
            <div class="context-usage-head" data-testid="context-usage-head"
                on:mousedown=move |ev| on_header_down.call(ev)
                on:dblclick=move |ev| on_header_dblclick.call(ev)>
                <h2 id="context-usage-title">{t(loc, "context_usage.title")}</h2>
                <div class="context-usage-head-actions">
                    {floating.then(|| view! {
                        <button type="button" class="context-usage-dock"
                            data-testid="context-usage-dock"
                            title=t(loc, "context_usage.dock")
                            aria-label=t(loc, "context_usage.dock")
                            on:click=move |_| on_dock.call(())>
                            {compose_icon("dock")}
                        </button>
                    })}
                    <button type="button" class="context-usage-close"
                        title=t(loc, "context_usage.close")
                        aria-label=t(loc, "context_usage.close")
                        on:click=move |_| context_usage_open.set(false)>
                        {compose_icon("close")}
                    </button>
                </div>
            </div>
            <div class="context-usage-summary">
                <span>{tf(loc, "context_usage.full", &[("pct", &pct.to_string())])}</span>
                <span>{total}</span>
            </div>
            {danger.then(|| view! {
                <div class="context-usage-nudge" data-testid="context-usage-nudge" role="status">
                    <span class="context-usage-nudge-copy">{t(loc, "context_usage.nudge")}</span>
                    <div class="context-usage-nudge-actions">
                        <button type="button" class="context-usage-nudge-action"
                            data-testid="context-usage-compact"
                            disabled=move || compact_disabled.get()
                            on:click=move |ev| {
                                ev.stop_propagation();
                                on_compact.call(());
                            }>
                            {t(loc, "context_usage.nudge_compact")}
                        </button>
                        <button type="button" class="context-usage-nudge-action"
                            data-testid="context-usage-new-session"
                            on:click=move |ev| {
                                ev.stop_propagation();
                                on_new_session.call(());
                            }>
                            {t(loc, "context_usage.nudge_new_session")}
                        </button>
                    </div>
                </div>
            })}
            <div class="context-usage-bar" role="img"
                aria-label=tf(loc, "context_usage.full", &[("pct", &pct.to_string())])>
                {segments.into_iter().filter(|row| row.tokens > 0).map(|row| {
                    let width = row.tokens as f64 * 100.0 / denominator as f64;
                    view! {
                        <span class=format!("context-usage-segment {}", row.color)
                            style=format!("width:{width:.4}%")></span>
                    }
                }).collect_view()}
            </div>
            <div class="context-usage-list">
                {rows.into_iter().map(|row| {
                    let expandable = row.color != "conversation";
                    let color = row.color.to_string();
                    let detail_color = color.clone();
                    let open_color = color.clone();
                    view! {
                        <div class="context-usage-item">
                            <button type="button" class="context-usage-row"
                                class:expandable=expandable
                                disabled=!expandable
                                aria-expanded=move || (expandable && context_usage_detail_open.get().as_deref() == Some(open_color.as_str())).to_string()
                                on:click=move |_| {
                                    if expandable {
                                        context_usage_detail_open.update(|active| {
                                            *active = (active.as_deref() != Some(color.as_str())).then(|| color.clone());
                                        });
                                    }
                                }>
                                <span class=format!("context-usage-swatch {}", row.color)
                                    aria-hidden="true"></span>
                                <span class="context-usage-label">{row.label}</span>
                                <span class="context-usage-value">{fmt_context_tokens(row.tokens)}</span>
                                {expandable.then(|| view! { <span class="context-usage-chevron">{"⌄"}</span> })}
                            </button>
                            {move || (context_usage_detail_open.get().as_deref() == Some(detail_color.as_str())).then(|| {
                                let content = context_usage_details.get()
                                    .map(|details| context_usage_detail_text(&details, &detail_color))
                                    .unwrap_or_else(|| t(locale.get(), "context_usage.loading").into());
                                view! { <pre class="context-usage-detail">{content}</pre> }
                            })}
                        </div>
                    }
                }).collect_view()}
            </div>
            {floating.then(|| view! {
                <button type="button" class="context-usage-resize"
                    data-testid="context-usage-resize"
                    title=t(loc, "context_usage.resize")
                    aria-label=t(loc, "context_usage.resize")
                    on:mousedown=move |ev| on_resize_start.call(ev)>
                </button>
            })}
        </section>
    }
}
