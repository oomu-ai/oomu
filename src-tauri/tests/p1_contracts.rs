use oomu_lib::p1_contracts::{
    ArtifactPresentation, ArtifactWorkbook, CapabilityBundle, DesktopAction, DesktopObservation,
    LearningCandidate, MediaAsset, RemoteDevice, WorkGraph,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

fn assert_fixture<T: DeserializeOwned>(contracts: &Value, name: &str) {
    serde_json::from_value::<T>(contracts[name].clone()).unwrap_or_else(|error| {
        panic!("{name} fixture did not satisfy the public Rust contract: {error}")
    });
}

#[test]
fn public_rust_contracts_accept_the_shared_p1_vectors() {
    let vectors: Value =
        serde_json::from_str(include_str!("../../schemas/p1-contract-v1-vectors.json"))
            .expect("shared P1 contract vectors must be valid JSON");
    let contracts = &vectors["contracts"];

    assert_fixture::<ArtifactWorkbook>(contracts, "artifactWorkbook");
    assert_fixture::<ArtifactPresentation>(contracts, "artifactPresentation");
    assert_fixture::<DesktopObservation>(contracts, "desktopObservation");
    assert_fixture::<DesktopAction>(contracts, "desktopAction");
    assert_fixture::<MediaAsset>(contracts, "mediaAsset");
    assert_fixture::<RemoteDevice>(contracts, "remoteDevice");
    assert_fixture::<CapabilityBundle>(contracts, "capabilityBundle");
    assert_fixture::<LearningCandidate>(contracts, "learningCandidate");
    assert_fixture::<WorkGraph>(contracts, "workGraph");
}
