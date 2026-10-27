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
    pub keep: bool,
    pub target_retained: bool,
    pub provider_retained: bool,
    pub from_provider: bool,
    pub requested: bool,
    pub video_source: Option<&'a str>,
    pub target: &'a str,
    pub uncertain: bool,
}
pub fn decide(p: Policy<'_>) -> Result<Effect> {
    if p.explicit {
        return Ok(Effect::None);
    }
    ensure!(
        !p.uncertain,
        "画面状态不确定，请手动恢复提供端画面并重新开始共享"
    );
    let source = p
        .video_source
        .ok_or_else(|| anyhow::anyhow!("尚未确定外接屏画面来源"))?;
    ensure!(
        !(p.keep
            && p.from_provider
            && p.provider_retained
            && !p.target_retained
            && !p.requested
            && source != p.target),
        "目标没有可用画面，已拦截边缘切换；请使用快捷键或按钮切换外接屏"
    );
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
            keep: true,
            target_retained: true,
            provider_retained: true,
            from_provider: false,
            requested: false,
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
    fn blocks_hidden_outer_edge_but_accepts_explicit_command() {
        let mut p = policy();
        p.target = "receiver";
        p.video_source = Some("provider");
        p.target_retained = false;
        p.from_provider = true;
        assert!(decide(p).is_err());
        let mut p = policy();
        p.explicit = true;
        assert_eq!(decide(p).unwrap(), Effect::None);
    }
    #[test]
    fn uncertainty_does_not_fall_back_to_keyboard_owner() {
        let mut p = policy();
        p.uncertain = true;
        p.video_source = None;
        assert!(decide(p).is_err());
        let mut p = policy();
        p.video_source = None;
        assert!(decide(p).is_err());
    }
    #[test]
    fn follow_mode_writes_even_with_retained_panel() {
        let mut p = policy();
        p.keep = false;
        assert_eq!(decide(p).unwrap(), Effect::Write);
    }
}
