//! Video routing depends on the last video command, never on keyboard ownership.
use anyhow::{ensure, Result};
#[derive(Debug, PartialEq)]
pub enum Effect {
    None,
    Write,
    Land,
}
pub struct Policy<'a> {
    pub explicit: bool,
    pub manual: bool,
    pub keep: bool,
    pub target_retained: bool,
    pub video_source: Option<&'a str>,
    pub target: &'a str,
    pub uncertain: bool,
}
pub fn decide(p: Policy<'_>) -> Result<Effect> {
    if p.explicit {
        return Ok(Effect::None);
    }
    if p.target_retained
        && (p.manual || p.keep)
        && (p.uncertain || p.video_source != Some(p.target))
    {
        return Ok(Effect::Land);
    }
    ensure!(
        !p.uncertain,
        "画面状态不确定，请手动恢复提供端画面并重新开始共享"
    );
    let source = p
        .video_source
        .ok_or_else(|| anyhow::anyhow!("尚未确定外接屏画面来源"))?;
    if p.manual {
        if source == p.target {
            return Ok(Effect::None);
        }
        ensure!(
            p.target_retained,
            "目标没有可见屏幕，已拦截边缘跨屏；请使用快捷键或按钮切换画面"
        );
        return Ok(Effect::Land);
    }
    // In keep mode an independent destination receives input only; a
    // shared-only destination explicitly opts into an edge-triggered DDC write.
    // Visibility gating belongs to manual mode above, not this policy.
    if source == p.target {
        Ok(Effect::None)
    } else if p.keep && p.target_retained {
        Ok(Effect::Land)
    } else {
        Ok(Effect::Write)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy<'static> {
        Policy {
            explicit: false,
            manual: false,
            keep: true,
            target_retained: true,
            video_source: Some("receiver"),
            target: "provider",
            uncertain: false,
        }
    }
    #[test]
    fn retains_video_for_keyboard_round_trip() {
        assert_eq!(decide(policy()).unwrap(), Effect::Land);
        let mut p = policy();
        p.target = "receiver";
        assert_eq!(decide(p).unwrap(), Effect::None);
    }
    #[test]
    fn keep_mode_switches_picture_for_shared_only_targets() {
        let mut p = policy();
        p.target = "receiver";
        p.video_source = Some("provider");
        p.target_retained = false;
        assert_eq!(decide(p).unwrap(), Effect::Write);
        let mut p = policy();
        p.target_retained = false;
        assert_eq!(decide(p).unwrap(), Effect::Write);
        let mut p = policy();
        p.explicit = true;
        assert_eq!(decide(p).unwrap(), Effect::None);
    }
    #[test]
    fn uncertainty_does_not_fall_back_to_keyboard_owner() {
        let mut p = policy();
        p.target_retained = false;
        p.uncertain = true;
        p.video_source = None;
        assert!(decide(p).is_err());
        let mut p = policy();
        p.target_retained = false;
        p.video_source = None;
        assert!(decide(p).is_err());
    }
    #[test]
    fn independent_screen_remains_reachable_when_shared_picture_is_unknown() {
        for manual in [false, true] {
            let mut p = policy();
            p.manual = manual;
            p.uncertain = true;
            p.video_source = None;
            assert_eq!(decide(p).unwrap(), Effect::Land);
        }
    }
    #[test]
    fn follow_mode_writes_even_with_retained_panel() {
        let mut p = policy();
        p.keep = false;
        assert_eq!(decide(p).unwrap(), Effect::Write);
    }
    #[test]
    fn manual_edges_never_write_video_in_either_direction() {
        for retained in [false, true] {
            let mut p = policy();
            p.manual = true;
            p.target_retained = retained;
            if retained {
                assert_eq!(decide(p).unwrap(), Effect::Land);
            } else {
                assert!(decide(p).is_err());
            }
        }
        let mut p = policy();
        p.manual = true;
        p.target = "receiver";
        p.target_retained = false;
        assert_eq!(decide(p).unwrap(), Effect::None);
        let mut p = policy();
        p.manual = true;
        p.explicit = true;
        p.target_retained = false;
        assert_eq!(decide(p).unwrap(), Effect::None);
    }
}
