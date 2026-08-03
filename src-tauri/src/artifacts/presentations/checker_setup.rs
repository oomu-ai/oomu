use super::{
    exact_package_preview::{presentation_checker_probe, presentation_checker_release},
    PresentationCheckerReadiness, PresentationCheckerStatus,
};

pub(crate) fn presentation_checker_readiness() -> PresentationCheckerReadiness {
    let (supported, candidate, engine, component) = presentation_checker_probe();
    let status = if !supported {
        PresentationCheckerStatus::UnsupportedPlatform
    } else if engine && component {
        PresentationCheckerStatus::Ready
    } else if engine {
        PresentationCheckerStatus::AppComponentUnavailable
    } else if candidate {
        PresentationCheckerStatus::NotQualified
    } else {
        PresentationCheckerStatus::NotInstalled
    };
    PresentationCheckerReadiness {
        status,
        required_version: presentation_checker_release().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_statuses_are_fail_closed() {
        let status = presentation_checker_readiness().status;
        assert!(matches!(
            status,
            PresentationCheckerStatus::Ready
                | PresentationCheckerStatus::NotInstalled
                | PresentationCheckerStatus::NotQualified
                | PresentationCheckerStatus::AppComponentUnavailable
                | PresentationCheckerStatus::UnsupportedPlatform
        ));
    }
}
