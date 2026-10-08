// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    adapter_catalog::AdapterCatalog,
    control_codec::{
        ControlCall, ControlLimits, ControlRequest, ControlResponse, ControlResult, ControlSuccess,
        NegotiateParams, Negotiated, ProviderCatalogAction, ProviderCatalogRequest, RequestId,
        Revision, SuccessResponse, V1_14, V1_15,
    },
    live_projection::{ControlProjection, DynamicProviderCatalogState},
    protocol_compatibility,
};

fn request(call: ControlCall, id: u64) -> ControlRequest {
    ControlRequest {
        jsonrpc: "2.0".into(),
        id: RequestId(id),
        timeout_ms: 1_000,
        call,
    }
}

fn negotiated(version: asb_tui::control_codec::ControlVersion) -> ControlResponse {
    ControlResponse::Success(SuccessResponse {
        jsonrpc: "2.0".into(),
        id: RequestId(1),
        result: ControlSuccess::Negotiated(Negotiated {
            version,
            limits: ControlLimits::default(),
            runner_instance_id: "runner-v115".into(),
            oldest_revision: Revision(1),
            latest_revision: Revision(7),
        }),
    })
}

#[test]
fn exact_asb_v115_fixture_projects_every_dynamic_model_into_wizard_choices() {
    let response: ControlResponse = serde_json::from_str(include_str!(
        "fixtures/asb-v1.15-dynamic-provider-catalog-response.json"
    ))
    .unwrap();
    let call = ControlCall::ProviderCatalog(ProviderCatalogRequest {
        action: ProviderCatalogAction::Refresh,
        runner_instance_id: "runner-v115".into(),
        known_generation: Some(Revision(6)),
    });
    response
        .validate_for(&request(call.clone(), 15), ControlLimits::default())
        .unwrap();

    let mut projection = ControlProjection::default();
    projection
        .apply(
            &request(
                ControlCall::Negotiate(NegotiateParams {
                    versions: [V1_15].into_iter().collect(),
                    limits: ControlLimits::default(),
                }),
                1,
            ),
            &negotiated(V1_15),
            ControlLimits::default(),
        )
        .unwrap();
    projection
        .apply(&request(call, 15), &response, ControlLimits::default())
        .unwrap();
    let snapshot = projection.snapshot();
    let DynamicProviderCatalogState::Catalog(dynamic) = snapshot.dynamic_provider_catalog.unwrap()
    else {
        panic!("expected v1.15 dynamic catalog");
    };
    assert_eq!(dynamic.openrouter.models.len(), 1);
    let adapters = AdapterCatalog::from_provider_catalog(&dynamic.catalog).unwrap();
    assert!(adapters.records().any(|record| {
        record
            .providers
            .get("openrouter")
            .is_some_and(|models| models.contains("example/free"))
    }));
}

#[test]
fn v114_remains_legacy_and_cannot_claim_dynamic_catalog_support() {
    assert!(protocol_compatibility::validate_version(V1_14).is_ok());
    assert!(V1_14 < V1_15);
    let encoded = include_str!("fixtures/asb-v1.15-dynamic-provider-catalog-response.json");
    let response: ControlResponse = serde_json::from_str(encoded).unwrap();
    let ControlResponse::Success(success) = response else {
        panic!("fixture is a success response");
    };
    let ControlSuccess::Operation(bound) = success.result else {
        panic!("fixture is an operation response");
    };
    assert!(matches!(
        bound.result,
        ControlResult::DynamicProviderCatalog(_)
    ));
}
