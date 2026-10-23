use std::rc::Rc;

use gpui_kit::TestAppContext;
use nocterm_project_model::{
    HostLocator, ProjectTarget, ProviderId, ProviderResourceRef, RootPath,
};

use crate::{
    ProjectConnectionError, ProjectProviderRegistry, ProjectTerminalResolve,
    test_support::ProjectTerminalProbe,
};

#[test]
fn duplicate_provider_registration_is_an_installation_error() {
    let direct: Rc<dyn ProjectTerminalResolve> = Rc::new(ProjectTerminalProbe::default());
    let id = ProviderId::parse("example.workspace").unwrap();
    let registry = ProjectProviderRegistry::new(
        direct.clone(),
        [(id.clone(), direct.clone()), (id.clone(), direct)],
    );
    assert!(
        matches!(registry, Err(ProjectConnectionError::DuplicateProvider(found)) if found == id)
    );
}

#[gpui_kit::test]
fn an_uninstalled_provider_never_falls_back_to_the_ordinary_host_resolver(cx: &mut TestAppContext) {
    let direct = Rc::new(ProjectTerminalProbe::default());
    let registry = ProjectProviderRegistry::new(direct.clone(), []).unwrap();
    let target = ProjectTarget::Provider {
        provider: ProviderId::parse("example.workspace").unwrap(),
        resource_version: 3,
        reference: ProviderResourceRef::new(serde_json::json!({ "resource": "remote-1" })).unwrap(),
    };
    cx.update(|cx| {
        assert!(matches!(
            registry.resolve(&target, cx),
            Err(ProjectConnectionError::ProviderUnavailable)
        ))
    });
    assert!(direct.resolutions().is_empty());
}

#[gpui_kit::test]
fn ordinary_folders_keep_the_explicit_direct_resolver(cx: &mut TestAppContext) {
    let direct = Rc::new(ProjectTerminalProbe::default());
    let registry = ProjectProviderRegistry::new(direct.clone(), []).unwrap();
    let target = ProjectTarget::HostFolder {
        host: HostLocator::Local,
        root: RootPath::parse("/workspace").unwrap(),
    };
    cx.update(|cx| {
        assert!(matches!(
            registry.resolve(&target, cx),
            Err(ProjectConnectionError::Unavailable)
        ))
    });
    assert_eq!(direct.resolutions(), vec![HostLocator::Local]);
}
