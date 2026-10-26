//! Message text that can be selected and copied, keeping its formatting.
//!
//! GPUI Kit's `SelectableText` takes plain text only; this is the same
//! element over a styled run (bold, code, links), joining the window-wide
//! text selection. Links open on a click that didn't select anything.

use std::ops::Range;

use gpui_kit::{
    base::{TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun},
    prelude::*,
    *,
};

pub(super) struct SelectableRich {
    id: ElementId,
    text: SharedString,
    styled: StyledText,
    links: Vec<(Range<usize>, String)>,
    selection: Hsla,
}

impl SelectableRich {
    pub(super) fn new(
        id: impl Into<ElementId>,
        text: SharedString,
        styled: StyledText,
        links: Vec<(Range<usize>, String)>,
        selection: Hsla,
    ) -> Self {
        Self {
            id: id.into(),
            text,
            styled,
            links,
            selection,
        }
    }
}

impl IntoElement for SelectableRich {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableRich {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text = self.text.clone();
        let handle = match global_id {
            Some(global_id) => {
                window.with_element_state(global_id, |retained: Option<TextSelectionHandle>, _| {
                    let handle = retained.unwrap_or_else(|| TextSelectionHandle::new(text, cx));
                    (handle.clone(), handle)
                })
            }
            None => TextSelectionHandle::new(text, cx),
        };
        let (layout_id, ()) = self
            .styled
            .request_layout(global_id, inspector_id, window, cx);
        (layout_id, handle)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled
            .prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        let selected_before = TextSelection::selected_text(window, cx);
        let projection = handle.update_runs(
            &[TextSelectionRun::new(
                self.text.clone(),
                layout.clone(),
                bounds,
            )],
            cx,
        );
        if selected_before != TextSelection::selected_text(window, cx) {
            window.refresh();
        }
        for range in projection.ranges().iter().flatten().cloned() {
            paint_selection(&layout, range, self.selection, window);
        }

        // Text cursor over text, a hand over links.
        let link_at = {
            let links = self.links.clone();
            let layout = layout.clone();
            move |position: Point<Pixels>| {
                let index = layout.index_for_position(position).ok()?;
                links
                    .iter()
                    .find(|(range, _)| range.contains(&index))
                    .map(|(_, url)| url.clone())
            }
        };
        let over_link = hitbox.is_hovered(window) && link_at(window.mouse_position()).is_some();
        window.set_cursor_style(
            if over_link {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            },
            hitbox,
        );
        if !self.links.is_empty() {
            let hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if !phase.bubble()
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                    || !TextSelection::selected_text(window, cx).is_empty()
                {
                    return;
                }
                if let Some(url) = link_at(event.position) {
                    cx.open_url(&url);
                }
            });
        }

        self.styled.paint(
            global_id,
            inspector_id,
            bounds,
            &mut (),
            &mut (),
            window,
            cx,
        );
    }
}

/// Selection highlight for `range`, line by line.
fn paint_selection(layout: &TextLayout, range: Range<usize>, color: Hsla, window: &mut Window) {
    let (Some(start), Some(end)) = (
        layout.position_for_index(range.start),
        layout.position_for_index(range.end),
    ) else {
        return;
    };
    let bounds = layout.bounds();
    let line = layout.line_height();
    let mut quads = Vec::new();
    if start.y == end.y {
        quads.push(Bounds::from_corners(start, point(end.x, end.y + line)));
    } else {
        quads.push(Bounds::from_corners(
            start,
            point(bounds.right(), start.y + line),
        ));
        if end.y > start.y + line {
            quads.push(Bounds::from_corners(
                point(bounds.left(), start.y + line),
                point(bounds.right(), end.y),
            ));
        }
        quads.push(Bounds::from_corners(
            point(bounds.left(), end.y),
            point(end.x, end.y + line),
        ));
    }
    for quad in quads {
        window.paint_quad(fill(quad, color));
    }
}
