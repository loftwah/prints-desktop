//! Precision viewing for the full view and for both comparison panes: Fit, true 100% device-pixel
//! inspection, bounded zoom/pan and a quiet control strip in the existing header.
//!
//! The geometry below follows the pinned Rune revision's public
//! `crates/rune-kit/src/image_zoom/math.rs`: `scale` is relative to the fitted image, `x`/`y`
//! offset the image centre from the viewport centre, and the natural zoom is capped at eight
//! times the source pixels. Keeping the small pure helpers local avoids a new dependency and the
//! kit preview's spring motion; every change lands at once, as the design calls for.
//!
//! A view stores its fitted size in *points* and its fit ratio in *device pixels per source
//! pixel*. At 100% the drawn width is `natural / device_scale` points, so Rune draws one source
//! texel per device pixel on both standard and high-DPI displays, without growing the source at
//! Fit.

use std::cell::{Cell, RefCell};

use rune::{
    Color, Cursor, Drag, El, Image, MagnifyEvent, Phase, Shape, SurfaceFit, Tx, WheelEvent, el, row,
};

use crate::{Ink, Studio, label};

const MAX_NATURAL_ZOOM: f32 = 8.0;
const DRAG_THRESHOLD: f32 = 4.0;
const KEY_STEP_POINTS: f32 = 24.0;
const MAX_CHECKER_CELLS: usize = 6000;

/// One √2 step, as the pinned Rune image preview uses.
pub(crate) const STEP: f32 = std::f32::consts::SQRT_2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ZoomView {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
}

pub(crate) const FIT_VIEW: ZoomView = ZoomView {
    scale: 1.0,
    x: 0.0,
    y: 0.0,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ZoomBounds {
    /// Fitted width in points, before zoom.
    pub width: f32,
    /// Fitted height in points, before zoom.
    pub height: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub max_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Camera {
    /// Fitted-relative zoom shared by linked panes.
    pub scale: f32,
    /// The source point held at the viewport centre, in 0..1 image coordinates.
    pub focal: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
struct Geometry {
    bounds: ZoomBounds,
    /// Device pixels drawn per source pixel at Fit; never above one.
    fit_ratio: f32,
}

fn max_scale(fit_ratio: f32) -> f32 {
    (MAX_NATURAL_ZOOM / fit_ratio.max(f32::EPSILON)).max(1.0)
}

/// Scales the view while the source point under `anchor` (viewport-centre points) stays put.
fn zoom_at(view: ZoomView, scale: f32, anchor: [f32; 2], bounds: &ZoomBounds) -> ZoomView {
    let next = scale.clamp(1.0, bounds.max_scale.max(1.0));
    let ratio = next / view.scale;
    clamp_view(
        ZoomView {
            scale: next,
            x: anchor[0] - (anchor[0] - view.x) * ratio,
            y: anchor[1] - (anchor[1] - view.y) * ratio,
        },
        bounds,
    )
}

fn pan_by(view: ZoomView, delta: [f32; 2], bounds: &ZoomBounds) -> ZoomView {
    clamp_view(
        ZoomView {
            scale: view.scale,
            x: view.x + delta[0],
            y: view.y + delta[1],
        },
        bounds,
    )
}

/// Keeps a zoomed image covering the viewport on axes it overflows, centred on the others.
fn clamp_view(view: ZoomView, bounds: &ZoomBounds) -> ZoomView {
    ZoomView {
        scale: view.scale,
        x: clamp_axis(view.x, bounds.width * view.scale, bounds.viewport_width),
        y: clamp_axis(view.y, bounds.height * view.scale, bounds.viewport_height),
    }
}

fn clamp_axis(offset: f32, size: f32, viewport: f32) -> f32 {
    let limit = ((size - viewport) / 2.0).max(0.0);
    // Adding zero turns -0 into 0.
    offset.clamp(-limit, limit) + 0.0
}

/// The pinned double-click target: 100% for a downscaled image, or twice Fit when 100% is barely
/// larger. The explicit 100% control uses `natural_scale` rather than this.
fn detail_scale(fit_ratio: f32) -> f32 {
    let actual = 1.0 / fit_ratio.max(f32::EPSILON);
    if actual >= 1.5 { actual } else { 2.0 }
}

fn wheel_zoom_factor(delta_y: f32) -> f32 {
    (-delta_y.clamp(-30.0, 30.0) / 100.0).exp()
}

/// Fit geometry for one pane. `device_scale` is device pixels per view point; the fit ratio is
/// capped at one so Fit never invents pixels a source doesn't have.
fn geometry(natural: [f32; 2], viewport: [f32; 2], device_scale: f32) -> Geometry {
    let scale = device_scale.max(f32::EPSILON);
    let [nw, nh] = natural.map(|side| side.max(1.0));
    let ratio = ((viewport[0].max(1.0) * scale) / nw)
        .min((viewport[1].max(1.0) * scale) / nh)
        .min(1.0)
        .max(f32::EPSILON);
    Geometry {
        bounds: ZoomBounds {
            width: nw * ratio / scale,
            height: nh * ratio / scale,
            viewport_width: viewport[0].max(1.0),
            viewport_height: viewport[1].max(1.0),
            max_scale: max_scale(ratio),
        },
        fit_ratio: ratio,
    }
}

/// The image's rect in pane-local points: `[x, y, width, height]`.
fn image_rect(bounds: &ZoomBounds, view: ZoomView) -> [f32; 4] {
    let view = clamp_view(view, bounds);
    let width = bounds.width * view.scale;
    let height = bounds.height * view.scale;
    [
        bounds.viewport_width / 2.0 + view.x - width / 2.0,
        bounds.viewport_height / 2.0 + view.y - height / 2.0,
        width,
        height,
    ]
}

/// The visible zoom, as a percentage of the source's natural pixels.
fn level(fit_ratio: f32, view: ZoomView) -> u32 {
    (view.scale * fit_ratio * 100.0).round() as u32
}

/// Fitted-relative scale that draws the source at exactly one texel per device pixel.
fn natural_scale(fit_ratio: f32) -> f32 {
    (1.0 / fit_ratio.max(f32::EPSILON)).clamp(1.0, max_scale(fit_ratio))
}

/// Reads a pane's camera as a normalized focal point. Different aspect ratios keep their own Fit,
/// so a shared raw point offset would drift; a normalized source point does not.
fn camera_from_view(view: ZoomView, bounds: &ZoomBounds) -> Camera {
    Camera {
        scale: view.scale,
        focal: [
            (0.5 - view.x / (bounds.width * view.scale).max(f32::EPSILON)).clamp(0.0, 1.0),
            (0.5 - view.y / (bounds.height * view.scale).max(f32::EPSILON)).clamp(0.0, 1.0),
        ],
    }
}

fn view_from_camera(camera: Camera, bounds: &ZoomBounds) -> ZoomView {
    let scale = camera.scale.clamp(1.0, bounds.max_scale.max(1.0));
    clamp_view(
        ZoomView {
            scale,
            x: -(camera.focal[0] - 0.5) * bounds.width * scale,
            y: -(camera.focal[1] - 0.5) * bounds.height * scale,
        },
        bounds,
    )
}

/// All viewer state uses cells so the immutable view can publish geometry and the event handlers
/// can move the camera without borrowing the whole `Studio`.
#[derive(Default)]
pub(crate) struct Viewer {
    view: [Cell<ZoomView>; 2],
    natural: [Cell<[f32; 2]>; 2],
    viewport: [Cell<[f32; 2]>; 2],
    drag_at: [Cell<[f32; 2]>; 2],
    dragged: [Cell<bool>; 2],
    device_scale: Cell<f32>,
    active: Cell<usize>,
    linked: Cell<Option<Camera>>,
    checker: Cell<bool>,
    checker_cache: RefCell<Option<([f32; 2], Color, Color, Shape)>>,
}

impl Viewer {
    /// The window's device scale, read from the view environment or a handler's `Tx`.
    pub(crate) fn set_device_scale(&self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.device_scale.set(scale);
        }
    }

    fn device_scale(&self) -> f32 {
        let scale = self.device_scale.get();
        if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        }
    }

    /// A new inspection starts fitted. The viewing background stays selected.
    pub(crate) fn reset(&mut self) {
        for pane in &self.view {
            pane.set(FIT_VIEW);
        }
        self.active.set(0);
        self.linked.set(None);
    }

    fn geometry(&self, pane: usize) -> Geometry {
        geometry(
            self.natural[pane].get(),
            self.viewport[pane].get(),
            self.device_scale(),
        )
    }

    /// Publish the pane's measured geometry. True when a rebuild is needed to draw it.
    fn measure(&self, pane: usize, natural: [f32; 2], viewport: [f32; 2]) -> bool {
        let changed = self.natural[pane].get() != natural || self.viewport[pane].get() != viewport;
        if !changed {
            return false;
        }
        self.natural[pane].set(natural);
        self.viewport[pane].set(viewport);
        let bounds = self.geometry(pane).bounds;
        self.view[pane].set(clamp_view(self.view[pane].get(), &bounds));
        true
    }

    fn viewport(&self, pane: usize) -> [f32; 2] {
        self.viewport[pane].get()
    }

    fn commit(&self, pane: usize, view: ZoomView, tx: &mut Tx) {
        let bounds = self.geometry(pane).bounds;
        let view = clamp_view(view, &bounds);
        if self.linked() {
            let mut camera = camera_from_view(view, &bounds);
            if let Some(previous) = self.linked.get() {
                if bounds.width * view.scale <= bounds.viewport_width {
                    camera.focal[0] = previous.focal[0];
                }
                if bounds.height * view.scale <= bounds.viewport_height {
                    camera.focal[1] = previous.focal[1];
                }
            }
            self.linked.set(Some(camera));
        } else {
            self.view[pane].set(view);
        }
        tx.instant();
    }

    fn view(&self, pane: usize) -> ZoomView {
        let bounds = self.geometry(pane).bounds;
        match self.linked.get() {
            Some(camera) => view_from_camera(camera, &bounds),
            None => clamp_view(self.view[pane].get(), &bounds),
        }
    }

    pub(crate) fn zoomed(&self, pane: usize) -> bool {
        self.view(pane).scale > 1.0 + 1e-4
    }

    fn level(&self, pane: usize) -> u32 {
        let geometry = self.geometry(pane);
        level(geometry.fit_ratio, self.view(pane))
    }

    /// The pane the pointer last touched; keyboard pan follows it without moving focus.
    pub(crate) fn active(&self) -> usize {
        self.active.get()
    }

    pub(crate) fn linked(&self) -> bool {
        self.linked.get().is_some()
    }

    pub(crate) fn checker(&self) -> bool {
        self.checker.get()
    }

    fn set_linked(&self, on: bool, tx: &mut Tx) {
        if on {
            let pane = self.active();
            self.linked.set(Some(camera_from_view(
                self.view(pane),
                &self.geometry(pane).bounds,
            )));
        } else {
            for pane in 0..self.view.len() {
                self.view[pane].set(self.view(pane));
            }
            self.linked.set(None);
        }
        tx.instant();
    }

    fn toggle_checker(&self, tx: &mut Tx) {
        self.checker.set(!self.checker.get());
        tx.instant();
    }

    fn zoom_by(&self, pane: usize, factor: f32, tx: &mut Tx) {
        let view = self.view(pane);
        let geometry = self.geometry(pane);
        self.commit(
            pane,
            zoom_at(view, view.scale * factor, [0.0, 0.0], &geometry.bounds),
            tx,
        );
    }

    pub(crate) fn zoom_in(&self, pane: usize, tx: &mut Tx) {
        self.zoom_by(pane, STEP, tx);
    }

    pub(crate) fn zoom_out(&self, pane: usize, tx: &mut Tx) {
        self.zoom_by(pane, 1.0 / STEP, tx);
    }

    pub(crate) fn fit(&self, pane: usize, tx: &mut Tx) {
        if self.linked() {
            self.linked.set(Some(Camera {
                scale: 1.0,
                focal: [0.5, 0.5],
            }));
            tx.instant();
        } else {
            self.commit(pane, FIT_VIEW, tx);
        }
    }

    pub(crate) fn natural(&self, pane: usize, tx: &mut Tx) {
        let geometry = self.geometry(pane);
        let view = self.view(pane);
        self.commit(
            pane,
            zoom_at(
                view,
                natural_scale(geometry.fit_ratio),
                [0.0, 0.0],
                &geometry.bounds,
            ),
            tx,
        );
    }

    /// Double-click toggles Fit and the pinned detail target about the pointer.
    fn detail(&self, pane: usize, at: [f32; 2], tx: &mut Tx) {
        let geometry = self.geometry(pane);
        let view = self.view(pane);
        let target = if view.scale > 1.0 + 1e-4 {
            1.0
        } else {
            detail_scale(geometry.fit_ratio)
        };
        self.commit(
            pane,
            zoom_at(
                view,
                target,
                [
                    at[0] - geometry.bounds.viewport_width / 2.0,
                    at[1] - geometry.bounds.viewport_height / 2.0,
                ],
                &geometry.bounds,
            ),
            tx,
        );
    }

    fn wheel(&self, pane: usize, event: WheelEvent, tx: &mut Tx) -> bool {
        self.active.set(pane);
        let view = self.view(pane);
        let geometry = self.geometry(pane);
        let at = [
            event.x - geometry.bounds.viewport_width / 2.0,
            event.y - geometry.bounds.viewport_height / 2.0,
        ];
        let next = if event.mods.ctrl || event.mods.cmd {
            zoom_at(
                view,
                view.scale * wheel_zoom_factor(-event.dy),
                at,
                &geometry.bounds,
            )
        } else if view.scale > 1.0 + 1e-4 {
            pan_by(view, [event.dx, event.dy], &geometry.bounds)
        } else {
            return false;
        };
        self.commit(pane, next, tx);
        true
    }

    fn magnify(&self, pane: usize, event: MagnifyEvent, tx: &mut Tx) -> bool {
        let view = self.view(pane);
        let geometry = self.geometry(pane);
        let factor = if event.wheel {
            -event.delta * 200.0
        } else {
            -(1.0 + event.delta).max(0.01).ln()
        };
        self.commit(
            pane,
            zoom_at(
                view,
                view.scale * wheel_zoom_factor(factor),
                [
                    event.x - geometry.bounds.viewport_width / 2.0,
                    event.y - geometry.bounds.viewport_height / 2.0,
                ],
                &geometry.bounds,
            ),
            tx,
        );
        true
    }

    fn drag(&self, pane: usize, drag: &Drag, tx: &mut Tx) {
        match drag.phase {
            Phase::Down => {
                self.active.set(pane);
                self.drag_at[pane].set([drag.dx, drag.dy]);
                self.dragged[pane].set(false);
            }
            Phase::Move => {
                let last = self.drag_at[pane].get();
                let delta = [drag.dx - last[0], drag.dy - last[1]];
                self.drag_at[pane].set([drag.dx, drag.dy]);
                if delta == [0.0, 0.0] {
                    return;
                }
                if drag.dx.hypot(drag.dy) > DRAG_THRESHOLD {
                    self.dragged[pane].set(true);
                }
                if self.zoomed(pane) {
                    let view = self.view(pane);
                    let geometry = self.geometry(pane);
                    self.commit(pane, pan_by(view, delta, &geometry.bounds), tx);
                }
            }
            Phase::Up => {
                if !drag.cancelled() && !self.dragged[pane].get() && drag.clicks == 2 {
                    self.detail(pane, [drag.x, drag.y], tx);
                }
                self.drag_at[pane].set([0.0, 0.0]);
                self.dragged[pane].set(false);
            }
        }
    }

    /// Arrow-key pan: 24 device points a press, only from an inspection screen.
    pub(crate) fn pan_key(&self, pane: usize, dx: f32, dy: f32, tx: &mut Tx) -> bool {
        if !self.zoomed(pane) {
            return false;
        }
        let step = KEY_STEP_POINTS;
        let view = self.view(pane);
        let geometry = self.geometry(pane);
        self.commit(
            pane,
            pan_by(view, [dx * step, dy * step], &geometry.bounds),
            tx,
        );
        true
    }

    fn dragging(&self, pane: usize) -> bool {
        self.dragged[pane].get()
    }

    /// A quiet, cached checkerboard; the cell grows when the pane is large so the element tree
    /// never carries thousands of quads.
    fn checker_shape(&self, pane: usize, base: Color, alt: Color) -> Option<Shape> {
        let size = self.viewport(pane);
        if size[0] <= 0.0 || size[1] <= 0.0 {
            return None;
        }
        if let Some((cached, cached_base, cached_alt, shape)) = self.checker_cache.borrow().as_ref()
            && *cached == size
            && *cached_base == base
            && *cached_alt == alt
        {
            return Some(shape.clone());
        }
        let mut tile = 12.0;
        while (size[0] / tile).ceil() as usize * (size[1] / tile).ceil() as usize
            > MAX_CHECKER_CELLS
        {
            tile *= 2.0;
        }
        let columns = (size[0] / tile).ceil() as usize;
        let rows = (size[1] / tile).ceil() as usize;
        let mut shape = Shape::new();
        for y in 0..rows {
            for x in 0..columns {
                if (x + y) % 2 != 0 {
                    continue;
                }
                let (left, top) = (x as f32 * tile, y as f32 * tile);
                shape = shape.fill(
                    vec![
                        [left, top],
                        [left + tile, top],
                        [left + tile, top + tile],
                        [left, top + tile],
                    ],
                    alt,
                );
            }
        }
        *self.checker_cache.borrow_mut() = Some((size, base, alt, shape.clone()));
        Some(shape)
    }
}

fn tool(
    key: &str,
    name: impl Into<String>,
    face: &str,
    p: Ink,
    handler: impl Fn(&mut Studio, &mut Tx) + 'static,
) -> El {
    el().key(key.to_owned())
        .role("button")
        .label(name.into())
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .cursor(Cursor::Pointer)
        .h(30.0)
        .px(9.0)
        .radius(6.0)
        .fill(p.field)
        .hover(p.line)
        .center()
        .child(label(face, 12.0, 600.0, p.primary))
        .on_press_tx(handler)
}

fn toggle(
    key: &str,
    name: &str,
    face: &str,
    on: bool,
    p: Ink,
    handler: impl Fn(&mut Studio, &mut Tx) + 'static,
) -> El {
    el().key(key.to_owned())
        .role("switch")
        .label(name.to_owned())
        .checked(on)
        .focusable(true)
        .focus_ring(2.0, 2.0, p.accent)
        .cursor(Cursor::Pointer)
        .h(30.0)
        .px(9.0)
        .radius(6.0)
        .fill(if on { p.accent } else { p.field })
        .hover(p.line)
        .center()
        .child(label(
            face,
            11.0,
            600.0,
            if on { p.on_accent } else { p.primary },
        ))
        .on_press_tx(handler)
}

/// The control strip lives in the view's existing header, so a loading or missing image never
/// moves it. The zoom level describes the active pane.
pub(crate) fn controls(viewer: &Viewer, p: Ink, pane: usize, with_link: bool) -> El {
    let level = format!("{}%", viewer.level(pane));
    let mut bar = row().key("viewer-controls").gap(5.0).items_center();
    bar = bar
        .child(tool(
            "viewer-zoom-out",
            "Zoom out",
            "−",
            p,
            move |s, tx| s.viewer.zoom_out(pane, tx),
        ))
        .child(label(&level, 12.0, 600.0, p.muted).min_w(40.0))
        .child(tool("viewer-zoom-in", "Zoom in", "+", p, move |s, tx| {
            s.viewer.zoom_in(pane, tx)
        }))
        .child(tool("viewer-fit", "Fit image", "Fit", p, move |s, tx| {
            s.viewer.fit(pane, tx)
        }))
        .child(tool(
            "viewer-100",
            "Zoom to original pixels",
            "100%",
            p,
            move |s, tx| s.viewer.natural(pane, tx),
        ));
    if with_link {
        let side = if pane == 0 { "Left" } else { "Right" };
        bar = bar.child(tool(
            "viewer-active-pane",
            format!("Inspect {side} image. Switch pane"),
            side,
            p,
            |s, _| {
                s.viewer.active.set(1 - s.viewer.active());
            },
        ));
        bar = bar.child(toggle(
            "viewer-link",
            "Linked zoom and pan",
            "Link views",
            viewer.linked(),
            p,
            |s, tx| {
                let on = !s.viewer.linked();
                s.viewer.set_linked(on, tx);
            },
        ));
    }
    bar.child(toggle(
        "viewer-background",
        "Checkerboard background",
        "Checkerboard",
        viewer.checker(),
        p,
        |s, tx| s.viewer.toggle_checker(tx),
    ))
}

/// One image pane. `pane` is 0 for full view and 0/1 for a comparison pair.
pub(crate) fn stage(
    viewer: &Viewer,
    image: Option<&Image>,
    available: bool,
    p: Ink,
    pane: usize,
) -> El {
    let natural = image
        .and_then(|image| image.intrinsic_size())
        .unwrap_or([1.0, 1.0]);
    let mut stage = el()
        .key(format!("viewer-pane-{pane}-{}x{}", natural[0], natural[1]))
        .w_full()
        .h_full()
        .fill(p.stage)
        .clip_rounded(10.0)
        .on_layout_tx(move |s: &mut Studio, [_, _, width, height], tx| {
            if s.viewer.measure(pane, natural, [width, height]) {
                tx.instant();
            }
        });
    if viewer.checker()
        && let Some(shape) = viewer.checker_shape(pane, p.stage, p.panel)
    {
        stage = stage.shape(shape);
    }
    if !available {
        return stage
            .center()
            .child(label("Original unavailable", 12.0, 500.0, p.muted));
    }
    if image.is_some_and(|image| image.error().is_some()) {
        return stage.center().child(label(
            "This image could not be opened.",
            14.0,
            500.0,
            p.muted,
        ));
    }
    let Some(image) = image.filter(|image| image.is_ready()) else {
        return stage
            .center()
            .child(label("Loading image…", 13.0, 500.0, p.muted));
    };
    let geometry = viewer.geometry(pane);
    let view = viewer.view(pane);
    let [x, y, width, height] = image_rect(&geometry.bounds, view);
    let at_natural = (view.scale - natural_scale(geometry.fit_ratio)).abs() < 1e-4;
    let picture = el()
        .absolute(x, y)
        .size(width, height)
        .pixels_fit(image.pixels(), SurfaceFit::Fit)
        // At 100% the element is exactly one device pixel per source texel; an unscaled place
        // keeps that mapping crisp even when the point size is fractional.
        .surface_unscaled(at_natural);
    stage = stage.child(picture);
    let lens = pane;
    stage = stage
        .on_drag(move |s: &mut Studio, drag: &mut Drag, tx: &mut Tx| s.viewer.drag(lens, drag, tx))
        .on_wheel(move |s: &mut Studio, event, tx: &mut Tx| s.viewer.wheel(lens, event, tx));
    if cfg!(target_os = "macos") {
        stage = stage.on_magnify(move |s: &mut Studio, event: MagnifyEvent, tx: &mut Tx| {
            s.viewer.magnify(lens, event, tx)
        });
    }
    stage.cursor(if viewer.zoomed(pane) {
        if viewer.dragging(pane) {
            Cursor::Grabbing
        } else {
            Cursor::Grab
        }
    } else {
        Cursor::Default
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn fit_holds_proportions_and_never_grows_the_source() {
        for natural in [[2400.0, 1800.0], [1200.0, 1200.0], [900.0, 2400.0]] {
            let g = geometry(natural, [800.0, 600.0], 2.0);
            let ratio = g.bounds.width / g.bounds.height;
            assert!(
                close(ratio, natural[0] / natural[1]),
                "source ratio {ratio}"
            );
            assert!(g.fit_ratio <= 1.0, "Fit never invents pixels");
            assert!(g.bounds.width <= 800.0 && g.bounds.height <= 600.0);
        }
    }

    #[test]
    fn one_to_one_maps_source_pixels_to_device_pixels() {
        for (natural, viewport, scale) in [
            ([1600.0, 1200.0], [800.0, 600.0], 2.0),
            ([800.0, 600.0], [800.0, 600.0], 2.0),
            ([4000.0, 3000.0], [800.0, 600.0], 2.0),
            ([1600.0, 1200.0], [800.0, 600.0], 1.5),
            ([1000.0, 1000.0], [500.0, 500.0], 1.25),
            ([1200.0, 2400.0], [800.0, 600.0], 2.0),
            ([1200.0, 2400.0], [800.0, 600.0], 1.0),
        ] {
            let g = geometry(natural, viewport, scale);
            let view = ZoomView {
                scale: natural_scale(g.fit_ratio),
                ..FIT_VIEW
            };
            assert_eq!(level(g.fit_ratio, view), 100);
            let rect = image_rect(&g.bounds, view);
            assert!(close(rect[2] * scale, natural[0]), "width {:?}", rect);
            assert!(close(rect[3] * scale, natural[1]), "height {:?}", rect);
        }
    }

    #[test]
    fn pan_and_zoom_stay_bounded() {
        let g = geometry([4000.0, 3000.0], [800.0, 600.0], 2.0);
        let zoomed = zoom_at(FIT_VIEW, g.bounds.max_scale, [0.0, 0.0], &g.bounds);
        let pushed = pan_by(zoomed, [100000.0, -100000.0], &g.bounds);
        let rect = image_rect(&g.bounds, pushed);
        assert!(rect[0] <= 0.0 + 0.01 && rect[0] + rect[2] >= 800.0 - 0.01);
        assert!(rect[1] <= 0.0 + 0.01 && rect[1] + rect[3] >= 600.0 - 0.01);
        assert_eq!(clamp_axis(-100.0, 100.0, 800.0), 0.0);
    }

    #[test]
    fn linked_camera_keeps_the_focal_point_across_aspect_ratios() {
        // Both panes overflow the viewport at this scale, so neither axis is re-centred by the
        // clamp that keeps a zoomed image covering its viewport.
        let wide = geometry([1200.0, 900.0], [600.0, 600.0], 2.0).bounds;
        let tall = geometry([900.0, 1200.0], [600.0, 600.0], 2.0).bounds;
        let view = zoom_at(FIT_VIEW, 2.0, [200.0, -100.0], &wide);
        let camera = camera_from_view(view, &wide);
        let other = view_from_camera(camera, &tall);
        assert!(close(camera.scale, other.scale));
        let back = camera_from_view(other, &tall);
        for axis in 0..2 {
            assert!(close(camera.focal[axis], back.focal[axis]), "focal drift");
        }
    }

    #[test]
    fn detail_scale_prefers_100_when_the_image_is_downscaled() {
        assert!(close(detail_scale(0.25), 4.0));
        assert!(close(detail_scale(0.9), 2.0));
        assert!(close(natural_scale(0.9), 1.0 / 0.9));
    }
}
