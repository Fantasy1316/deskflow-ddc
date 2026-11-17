//! Focus and drag transitions are independent of file preparation/transfer.
#[derive(Default)]
pub(super) struct Interaction {
    pub tray_drag: bool,
    pub panel_drag: bool,
    picking: bool,
    visible: bool,
    presentation: u64,
    revision: u64,
    deferred_blur: bool,
}
impl Interaction {
    pub fn open(&mut self) {
        self.visible = true;
        self.presentation += 1;
        self.focused();
    }
    pub fn focused(&mut self) {
        self.revision += 1;
        self.deferred_blur = false;
    }
    pub fn begin_picker(&mut self) -> Option<u64> {
        if !self.visible || self.picking {
            return None;
        }
        // Revoke even a blur queued before the picker command reached Rust.
        self.focused();
        self.picking = true;
        Some(self.presentation)
    }
    pub fn finish_picker(&mut self, presentation: u64) -> bool {
        self.picking = false;
        self.focused();
        self.visible && self.presentation == presentation
    }
    pub fn dragging(&self) -> bool {
        self.tray_drag || self.panel_drag
    }
    pub fn blur(&mut self) -> Option<u64> {
        // A dialog temporarily owns focus; it is not a request to dismiss the
        // parent later. In particular, do not replay it when selection ends.
        if self.picking || !self.visible {
            return None;
        }
        self.deferred_blur = true;
        self.dismiss_ticket()
    }
    pub fn dismiss_ticket(&self) -> Option<u64> {
        (self.visible && self.deferred_blur && !self.dragging() && !self.picking)
            .then_some(self.revision)
    }
    pub fn can_dismiss(&self, ticket: u64) -> bool {
        self.dismiss_ticket() == Some(ticket)
    }
    pub fn dropped(&mut self) {
        self.tray_drag = false;
        self.panel_drag = false;
        self.focused();
    }
    pub fn hide(&mut self) {
        self.visible = false;
        self.presentation += 1;
        self.dropped();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blur_waits_across_the_tray_to_panel_gap_and_cannot_close_a_new_open() {
        let mut state = Interaction::default();
        state.open();
        state.tray_drag = true;
        assert!(state.blur().is_none());
        state.tray_drag = false;
        let old = state.dismiss_ticket().unwrap();
        state.panel_drag = true;
        assert!(!state.can_dismiss(old));
        state.panel_drag = false;
        assert!(state.can_dismiss(old));
        state.open();
        assert!(!state.can_dismiss(old));
    }
    #[test]
    fn picking_and_active_drag_protect_focus_but_preparing_does_not() {
        let mut state = Interaction::default();
        state.open();
        let picker = state.begin_picker().unwrap();
        assert!(state.blur().is_none());
        assert!(state.finish_picker(picker));
        assert!(state.dismiss_ticket().is_none());
        assert!(state.blur().is_some());
        state.panel_drag = true;
        assert!(state.dismiss_ticket().is_none());
        state.dropped();
        assert!(state.dismiss_ticket().is_none());
        assert!(state.blur().is_some());
    }
    #[test]
    fn picker_focus_never_replays_blur_before_during_or_after_selecting_or_cancelling() {
        // Completion/cancellation may return key focus before or after the
        // dialog result; neither ordering may replay its blur.
        for focus_returns_first in [false, true] {
            let mut state = Interaction::default();
            state.open();
            let queued = state.blur().unwrap();
            let picker = state.begin_picker().unwrap();
            assert!(!state.can_dismiss(queued));
            assert!(state.begin_picker().is_none());
            assert!(state.blur().is_none());
            if focus_returns_first {
                state.focused();
            }
            assert!(state.blur().is_none());
            assert!(state.finish_picker(picker));
            assert!(state.dismiss_ticket().is_none());
            assert!(!state.can_dismiss(queued));
            // A subsequent real outside click still dismisses normally.
            let outside = state.blur().unwrap();
            assert!(state.can_dismiss(outside));
        }
    }
    #[test]
    fn finishing_a_picker_never_reopens_or_focuses_a_closed_or_replaced_panel() {
        let mut state = Interaction::default();
        assert!(state.begin_picker().is_none());
        state.open();
        let closed = state.begin_picker().unwrap();
        state.hide();
        state.focused();
        assert!(!state.finish_picker(closed));
        assert!(state.blur().is_none());
        state.open();
        let replaced = state.begin_picker().unwrap();
        state.hide();
        state.open();
        assert!(!state.finish_picker(replaced));
        let next = state.begin_picker().unwrap();
        assert!(state.finish_picker(next));
    }
}
