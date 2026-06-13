use objc2::rc::Retained;
use objc2_foundation::NSString;
use objc2_quartz_core::{CABasicAnimation, CAMediaTiming, CATransition};

pub fn fade_transition(duration_s: f64) -> Retained<CATransition> {
    let transition = CATransition::animation();
    transition.setDuration(duration_s);
    transition
}

pub fn basic_animation(key_path: &NSString, duration_s: f64) -> Retained<CABasicAnimation> {
    let animation = CABasicAnimation::animationWithKeyPath(Some(key_path));
    animation.setDuration(duration_s);
    animation
}
