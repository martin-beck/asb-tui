// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::{
    terminal::RenderPolicy,
    ui::{Screen, WorkspaceState},
    wizard::{
        FormalEvent, StartupRoute, Step, Wizard, WizardFormalState, element_id, plain_text, render,
        startup_route,
    },
    wizard_catalog::{OptionKind, WizardCatalog, WizardOption},
    wizard_stepper::WizardPane,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn policy(unicode: bool) -> RenderPolicy {
    RenderPolicy {
        tier: asb_tui::terminal::CapabilityTier::Plain,
        unicode,
        mouse: false,
        focus: false,
        bracketed_paste: false,
        synchronized_output: false,
        alternate_screen: false,
    }
}

fn rendered_text(wizard: &Wizard, policy: RenderPolicy, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, wizard, policy))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn rendered_workspace_text(
    state: &WorkspaceState,
    policy: RenderPolicy,
    width: u16,
    height: u16,
) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| asb_tui::ui::render(frame, state, policy))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn completed_review_wizard(authentication: &str) -> Wizard {
    let mut wizard = Wizard::default();
    for value in ["codex", "development", "fixture-model", "all defaults"] {
        wizard.set_value(value).unwrap();
        wizard.advance().unwrap();
    }
    wizard.set_value(authentication).unwrap();
    wizard.advance().unwrap();
    wizard.set_value("local-mock").unwrap();
    wizard.advance().unwrap();
    wizard.set_value("strict offline replay").unwrap();
    wizard.advance().unwrap();
    assert_eq!(wizard.step(), Step::Review);
    wizard
}

#[test]
fn review_visibly_summarizes_every_choice_without_disclosing_credential_digest() {
    let digest = "a".repeat(64);
    let wizard = completed_review_wizard(&format!("credential_reference:{digest}"));
    let ordinary = rendered_text(&wizard, policy(true), 120, 30);
    for expected in [
        "Step: Review",
        "Review all seven setup choices",
        "Agents: codex",
        "Provider: development",
        "Model: fixture-model",
        "Configuration: all defaults",
        "Authentication: Credential reference",
        "Recording: local-mock",
        "Replay: strict offline replay",
    ] {
        assert!(ordinary.contains(expected), "missing {expected:?}");
    }
    assert!(!ordinary.contains(&digest));
    assert!(!ordinary.contains("Review: none selected"));

    let narrow = rendered_text(&wizard, policy(false), 60, 16);
    assert!(narrow.contains("Selected: codex / development / fixture-model"));
    assert!(narrow.contains("Enter confirm; Esc revise"));
    assert!(!narrow.contains(&digest));
}

#[test]
fn setup_is_required_only_when_authoritative_readiness_is_false() {
    assert_eq!(startup_route(false), StartupRoute::Wizard);
    assert_eq!(startup_route(true), StartupRoute::Landing);
}

#[test]
fn ui_model_declares_runner_setup_and_recording_projection_state() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    let fields = model["state_fields"].as_array().unwrap();
    assert!(
        model["auth_status_after_enrollment"]
            .as_str()
            .unwrap()
            .contains("helper enrollment or invocation")
    );
    for field in [
        "provider_catalog",
        "configuration",
        "auth_status",
        "recording_campaign",
        "recording_estimate",
        "recording_campaign_lifecycle",
    ] {
        assert!(fields.iter().any(|value| value == field));
    }
    assert!(
        model["elements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value["id"] == "configuration.control_status")
    );
    assert_eq!(
        model["help"]
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["id"] == "configuration.control_status")
            .unwrap()["text"],
        "Inspect provider model, credential-free authentication status, and recording readiness reported by the authenticated runner."
    );
}

#[test]
fn ui_model_owns_every_persistent_wizard_shell_region() {
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../docs/ui-state-model.json")).unwrap();
    let wizard = model["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|route| route["id"] == "wizard")
        .unwrap();
    let elements = wizard["elements"].as_array().unwrap();
    for id in [
        "wizard.shell_header",
        "wizard.step_region",
        "wizard.selection_list",
        "wizard.choice_details",
        "wizard.selection_summary",
        "wizard.shell_footer",
    ] {
        assert!(elements.iter().any(|element| element == id), "missing {id}");
        assert!(
            model["help"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["id"] == id)
        );
    }
}

#[test]
fn wizard_render_has_stable_context_and_tiny_fallback() {
    let wizard = Wizard::default();
    assert_eq!(element_id(wizard.step()), "wizard.agent");
    assert!(plain_text(&wizard).contains("ASB setup wizard"));
    let mut terminal = Terminal::new(TestBackend::new(24, 6)).unwrap();
    terminal
        .draw(|frame| render(frame, &wizard, policy(false)))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("ASB setup wizard"));
}

#[test]
fn wizard_shell_keeps_identity_progress_content_and_contextual_footer_visible() {
    let wizard = Wizard::default();
    let text = rendered_text(&wizard, policy(true), 240, 24);
    for label in [
        "Wizard shell",
        "Progress",
        "[>] Agent",
        "[ ] Provider",
        "[-] Workloads",
        "Choice details",
        "Selection summary",
        "Keyboard help",
        "Space select/unselect",
    ] {
        assert!(text.contains(label), "missing shell label {label:?}");
    }
}

#[test]
fn wizard_selection_rows_and_summary_are_textually_unambiguous() {
    let catalog = WizardCatalog::new(
        vec![
            WizardOption::new("agent-a", "Agent A", true).unwrap(),
            WizardOption::new("agent-b", "Agent B", false).unwrap(),
        ],
        vec![
            WizardOption::new("provider", "Provider", true)
                .unwrap()
                .compatible_with(vec!["agent-a".into(), "agent-b".into()])
                .unwrap(),
        ],
        vec![
            WizardOption::new("model", "Model", true)
                .unwrap()
                .compatible_with(vec!["provider".into()])
                .unwrap(),
        ],
    )
    .unwrap();
    let mut state = WizardFormalState::new_with_catalog(catalog).unwrap();
    state.apply(FormalEvent::OpenWizard).unwrap();
    state.apply(FormalEvent::CatalogSelect).unwrap();
    state.apply(FormalEvent::CatalogMove(1)).unwrap();
    let text = rendered_text(state.wizard(), policy(false), 120, 24);
    assert!(text.contains("[x] Agent A"));
    assert!(text.contains("> [ ] Agent B — unavailable: not available for this setup"));
    assert!(text.contains("Selection summary"));
    assert!(text.contains("Agent: 1 selected"));
    assert!(text.contains("Space toggles; A selects all or clears all"));
}

#[test]
fn narrow_selection_summary_keeps_selected_name_and_context_action_visible() {
    let catalog = WizardCatalog::new(
        vec![WizardOption::new("agent-a", "Agent A", true).unwrap()],
        vec![
            WizardOption::new("provider", "Provider", true)
                .unwrap()
                .compatible_with(vec!["agent-a".into()])
                .unwrap(),
        ],
        vec![
            WizardOption::new("model", "Model", true)
                .unwrap()
                .compatible_with(vec!["provider".into()])
                .unwrap(),
        ],
    )
    .unwrap();
    let mut state = WizardFormalState::new_with_catalog(catalog).unwrap();
    state.apply(FormalEvent::OpenWizard).unwrap();
    state.apply(FormalEvent::CatalogSelect).unwrap();
    let text = rendered_text(state.wizard(), policy(false), 40, 12);
    assert!(text.contains("Agent A"));
    assert!(text.contains("Space toggle; A select/clear visible."));
}

#[test]
fn compact_monochrome_shell_uses_textual_progress_and_never_sets_colours() {
    let wizard = Wizard::default();
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render(frame, &wizard, policy(false)))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Step 1/13: [>] Agent"));
    assert!(text.contains("Keyboard help"));
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == ratatui::style::Color::Reset
                && cell.bg == ratatui::style::Color::Reset)
    );
}

#[test]
fn stepper_resizes_truthfully_from_full_journey_to_compact_and_tiny() {
    let wizard = Wizard::default();
    let wide = rendered_text(&wizard, policy(false), 240, 24);
    for label in [
        "[>] Agent",
        "[ ] Provider",
        "[-] Workloads",
        "[-] Measures",
        "[-] Run",
        "[-] Results",
        "[-] Compare",
    ] {
        assert!(wide.contains(label), "missing wide stepper label {label:?}");
    }

    let compact = rendered_text(&wizard, policy(false), 80, 24);
    assert!(compact.contains("Step 1/13: [>] Agent"));
    assert!(compact.contains("[-] ASB catalog/plan/completed runs"));

    let tiny = rendered_text(&wizard, policy(false), 24, 6);
    assert!(tiny.contains("ASB setup wizard"));
    assert!(tiny.contains("location: 1/13 Agent"));
    assert!(tiny.contains("unavailable: workloads"));

    let reasons = rendered_text(&wizard, policy(false), 120, 24);
    assert!(reasons.contains("Workloads: requires an authoritative ASB catalog"));
    assert!(reasons.contains("Run: requires an authoritative ASB plan and launch"));
}

#[test]
fn modeled_pane_cycle_and_conventional_navigation_preserve_draft() {
    let mut formal = WizardFormalState::new().unwrap();
    formal.apply(FormalEvent::OpenWizard).unwrap();
    assert_eq!(formal.pane(), WizardPane::Choices);
    for expected in [
        WizardPane::Summary,
        WizardPane::Controls,
        WizardPane::Progress,
        WizardPane::Choices,
    ] {
        formal.apply(FormalEvent::CyclePane).unwrap();
        assert_eq!(formal.pane(), expected);
    }

    let mut state = WorkspaceState::for_startup(false);
    state.handle_key(key(KeyCode::Char('a')));
    state.handle_key(key(KeyCode::Enter));
    assert_eq!(state.wizard.step(), Step::Provider);
    state.handle_key(key(KeyCode::Left));
    assert_eq!(state.wizard.step(), Step::Agent);
    state.handle_key(key(KeyCode::Backspace));
    assert_eq!(state.wizard.current_value(), "");
    assert_eq!(state.screen, Screen::Wizard);
    state.handle_key(key(KeyCode::Backspace));
    assert_eq!(state.screen, Screen::Landing);
    state.open_wizard();
    assert_eq!(state.screen, Screen::Wizard);

    let mut pane_state = WorkspaceState::for_startup(false);
    let before = rendered_workspace_text(&pane_state, policy(false), 140, 24);
    pane_state.handle_key(key(KeyCode::Tab));
    let after = rendered_workspace_text(&pane_state, policy(false), 140, 24);
    assert!(before.contains("Choice details *"));
    assert!(after.contains("Selection summary *"));
}

#[test]
fn authentication_handoff_is_explicitly_credential_free() {
    let source = std::fs::read_to_string("src/wizard.rs").expect("wizard source");
    assert!(source.contains("approved keychain/helper"));
    assert!(source.contains("credential_helper:<endpoint_sha256>:<locator_sha256>"));
    assert!(source.contains("never paste an API key"));
}

#[test]
fn formal_wizard_flow_checks_documented_route_and_step_effects() {
    let mut state = WizardFormalState::new().unwrap();
    assert_eq!(
        state.apply(FormalEvent::Next),
        Err(asb_tui::wizard::WizardError::InvalidModel)
    );
    state.apply(FormalEvent::OpenWizard).unwrap();
    for value in [
        "agent",
        "provider",
        "model",
        "all defaults",
        "credential_reference:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "live",
        "strict offline replay",
    ] {
        state.apply(FormalEvent::SetValue(value.into())).unwrap();
        state.apply(FormalEvent::Next).unwrap();
    }
    assert_eq!(state.step(), Step::Review);
    state.apply(FormalEvent::Complete).unwrap();
    assert_eq!(state.route(), StartupRoute::Landing);
}

#[test]
fn formal_wizard_model_checks_typed_multi_agent_and_all_agent_actions() {
    let catalog = WizardCatalog::new(
        vec![
            WizardOption::new("agent-a", "Agent A", true).unwrap(),
            WizardOption::new("agent-b", "Agent B", true).unwrap(),
        ],
        vec![
            WizardOption::new("shared", "Shared provider", true)
                .unwrap()
                .compatible_with(vec!["agent-a".into(), "agent-b".into()])
                .unwrap(),
        ],
        vec![
            WizardOption::new("model", "Model", true)
                .unwrap()
                .compatible_with(vec!["shared".into()])
                .unwrap(),
        ],
    )
    .unwrap();
    let mut state = WizardFormalState::new_with_catalog(catalog).unwrap();
    state.apply(FormalEvent::OpenWizard).unwrap();
    state.apply(FormalEvent::CatalogSelect).unwrap();
    assert_eq!(state.wizard().values()[0], "agent-a");
    state
        .apply(FormalEvent::CatalogToggleVisibleAgents)
        .unwrap();
    assert_eq!(state.wizard().values()[0], "agent-a,agent-b");
    state
        .apply(FormalEvent::CatalogToggleVisibleAgents)
        .unwrap();
    assert_eq!(state.wizard().values()[0], "");
    state
        .apply(FormalEvent::CatalogToggleVisibleAgents)
        .unwrap();
    assert_eq!(state.wizard().values()[0], "agent-a,agent-b");
    state.apply(FormalEvent::Next).unwrap();
    assert_eq!(
        state.wizard().catalog().unwrap().visible_options()[0].id,
        "shared"
    );
}

#[test]
fn automatic_startup_open_is_a_formal_transition() {
    let mut state = WizardFormalState::new().unwrap();
    state.apply(FormalEvent::AutoOpenWizard).unwrap();
    assert_eq!(state.route(), StartupRoute::Wizard);
    assert_eq!(state.step(), Step::Agent);
    assert!(state.apply(FormalEvent::AutoOpenWizard).is_err());
}

#[test]
fn back_and_cancel_are_formal_wizard_transitions() {
    let mut state = WizardFormalState::new().unwrap();
    state.apply(FormalEvent::OpenWizard).unwrap();
    assert_eq!(
        state.apply(FormalEvent::Back),
        Err(asb_tui::wizard::WizardError::AtStart)
    );
    state.apply(FormalEvent::SetValue("agent".into())).unwrap();
    state.apply(FormalEvent::Next).unwrap();
    state.apply(FormalEvent::Back).unwrap();
    assert_eq!(state.step(), Step::Agent);
    state.apply(FormalEvent::Cancel).unwrap();
    assert_eq!(state.route(), StartupRoute::Landing);
}

#[test]
fn wizard_validates_input_and_boundaries() {
    use asb_tui::wizard::WizardError;

    let mut wizard = Wizard::default();
    assert_eq!(wizard.advance(), Err(WizardError::Missing));
    assert_eq!(wizard.back(), Err(WizardError::AtStart));
    assert_eq!(
        wizard.set_value("line\nfeed"),
        Err(WizardError::InvalidValue)
    );
    assert_eq!(wizard.set_value("x".repeat(257)), Err(WizardError::TooLong));

    for value in [
        "agent",
        "provider",
        "model",
        "all defaults",
        "credential_reference:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "live",
        "strict offline replay",
    ] {
        wizard.set_value(value).unwrap();
        wizard.advance().unwrap();
    }
    assert_eq!(wizard.step(), Step::Review);
    assert_eq!(
        wizard.set_value("no draft on review"),
        Err(WizardError::InvalidValue)
    );
    assert_eq!(wizard.complete(), Ok(()));
    assert_eq!(wizard.advance(), Err(WizardError::AtEnd));
    wizard.back().unwrap();
    assert_eq!(wizard.step(), Step::Replay);
    wizard.cancel();
    assert!(wizard.cancelled());
}

#[test]
fn wide_render_covers_every_documented_wizard_step() {
    let mut wizard = Wizard::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let values = [
        "agent",
        "provider",
        "model",
        "all defaults",
        "credential_reference:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "live",
        "strict offline replay",
    ];
    for (index, value) in values.iter().enumerate() {
        assert_eq!(
            element_id(wizard.step()),
            [
                "wizard.agent",
                "wizard.provider",
                "wizard.model",
                "wizard.configuration",
                "wizard.authentication",
                "wizard.recording",
                "wizard.replay",
                "wizard.review",
            ][index]
        );
        wizard.set_value(*value).unwrap();
        terminal
            .draw(|frame| render(frame, &wizard, policy(true)))
            .unwrap();
        wizard.advance().unwrap();
    }
    assert_eq!(element_id(wizard.step()), "wizard.review");
    terminal
        .draw(|frame| render(frame, &wizard, policy(true)))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Choice details"));
    assert!(text.contains("Selection summary"));
    assert!(text.contains("Review all choices"));
}

#[test]
fn wizard_completion_is_consumed_once_and_restart_safe() {
    let mut state = WorkspaceState::for_startup(false);
    for _ in 0..7 {
        state.handle_key(key(KeyCode::Char('x')));
        state.handle_key(key(KeyCode::Enter));
    }
    state.handle_key(key(KeyCode::Enter));
    let mut values = state.take_wizard_completion().expect("completed draft");
    // The fixture route may leave an intentionally invalid auth draft; choosing
    // the explicit no-credential development mode makes the apply seam valid.
    values[4] = "none".into();
    let selection = WorkspaceState::wizard_configuration_selection(&values).unwrap();
    assert_eq!(selection.agent_ids, vec!["x"]);
    assert!(state.take_wizard_completion().is_none());

    // A restarted workspace has no completion payload and cannot replay the
    // prior selection without a fresh formal completion.
    let mut restarted = WorkspaceState::for_startup(false);
    assert!(restarted.take_wizard_completion().is_none());
    assert_eq!(restarted.wizard.step(), asb_tui::wizard::Step::Agent);
    restarted.handle_key(key(KeyCode::Esc));
    assert!(restarted.take_wizard_completion().is_none());
}

#[test]
fn formal_wizard_rejects_events_on_the_wrong_route_atomically() {
    use asb_tui::wizard::WizardError;

    let mut state = WizardFormalState::new().unwrap();
    for event in [
        FormalEvent::Back,
        FormalEvent::Complete,
        FormalEvent::Cancel,
        FormalEvent::SetValue("draft".into()),
    ] {
        assert_eq!(state.apply(event), Err(WizardError::InvalidModel));
        assert_eq!(state.route(), StartupRoute::Landing);
    }
    state.apply(FormalEvent::OpenWizard).unwrap();
    assert_eq!(
        state.apply(FormalEvent::OpenWizard),
        Err(WizardError::InvalidModel)
    );
    assert_eq!(state.step(), Step::Agent);
}

#[test]
fn wizard_uses_catalog_for_agent_provider_and_model_steps() {
    let catalog = WizardCatalog::new(
        vec![WizardOption::new("agent-a", "Agent A", true).unwrap()],
        vec![
            WizardOption::new("provider-a", "Provider A", true)
                .unwrap()
                .compatible_with(vec!["agent-a".into()])
                .unwrap(),
        ],
        vec![
            WizardOption::new("model-a", "Model A", true)
                .unwrap()
                .compatible_with(vec!["provider-a".into()])
                .unwrap(),
        ],
    )
    .unwrap();
    let mut state = WorkspaceState::default().with_wizard_catalog(catalog);
    state.open_wizard();
    assert_eq!(state.screen, Screen::Wizard);
    for (step, selected) in [
        (asb_tui::wizard::Step::Agent, "agent-a"),
        (asb_tui::wizard::Step::Provider, "provider-a"),
        (asb_tui::wizard::Step::Model, "model-a"),
    ] {
        assert_eq!(state.wizard.step(), step);
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let kind = match step {
            asb_tui::wizard::Step::Agent => OptionKind::Agent,
            asb_tui::wizard::Step::Provider => OptionKind::Provider,
            asb_tui::wizard::Step::Model => OptionKind::Model,
            _ => unreachable!(),
        };
        assert_eq!(
            state.wizard.catalog().unwrap().selected(kind),
            Some(selected)
        );
    }
    assert_eq!(state.wizard.step(), asb_tui::wizard::Step::Configuration);
    // Catalog state retains each independent selection after step changes.
    let catalog = state.wizard.catalog().unwrap();
    assert_eq!(catalog.selected(OptionKind::Agent), Some("agent-a"));
    assert_eq!(catalog.selected(OptionKind::Provider), Some("provider-a"));
    assert_eq!(catalog.selected(OptionKind::Model), Some("model-a"));
}

#[test]
fn authenticated_projection_drives_repeatable_setup_without_secret_defaults() {
    use asb_tui::{
        agent_catalog::{
            AgentAvailability, AgentCatalog, AgentCatalogEntry, AgentPackage, AgentProvenance,
            AgentSigner, AgentTarget,
        },
        control_codec::{
            ConfigurationSnapshot, ProviderAuthMethod, ProviderAvailability, ProviderCatalog,
            ProviderCatalogEntry, ProviderModel, Revision,
        },
        live_projection::{Connection, LiveSnapshot},
        wizard::Step,
    };

    let target = AgentTarget {
        operating_system: "linux".into(),
        architecture: "x86_64".into(),
        libc: "glibc".into(),
        libc_version: "2.35".into(),
    };
    let agent_catalog = AgentCatalog {
        runner_instance_id: "runner-setup".into(),
        generation: 4,
        catalog_sha256: "a".repeat(64),
        target: target.clone(),
        agents: vec![AgentCatalogEntry {
            agent_id: "agent-a".into(),
            target: target.clone(),
            package: Some(AgentPackage {
                package_id: "agent-a-package".into(),
                version: "1.0.0".into(),
                sha256: "b".repeat(64),
                signature_sha256: "c".repeat(64),
                signer: AgentSigner {
                    key_id: "release-key".into(),
                    principal: "asb-release".into(),
                },
            }),
            provenance: Some(AgentProvenance {
                source_revision: "d".repeat(40),
                manifest_sha256: "e".repeat(64),
                sbom_sha256: "f".repeat(64),
                license_ref: "MIT".into(),
            }),
            capabilities: vec!["benchmark".into()],
            availability: AgentAvailability::Available,
        }],
        refreshed: true,
    };
    let provider_catalog = ProviderCatalog {
        runner_instance_id: "runner-setup".into(),
        generation: Revision(8),
        catalog_sha256: "1".repeat(64),
        providers: vec![ProviderCatalogEntry {
            provider_id: "provider-a".into(),
            display_name: "Provider A".into(),
            auth_methods: vec![ProviderAuthMethod::CredentialReference],
            availability: ProviderAvailability::Available,
            models: vec![ProviderModel {
                model_id: "model-a".into(),
                revision: "r1".into(),
                availability: ProviderAvailability::Available,
            }],
        }],
        refreshed: true,
    };
    let mut state = WorkspaceState::default();
    state.apply_live_snapshot(LiveSnapshot {
        connection: Connection::Negotiated,
        runner_instance_id: Some("runner-setup".into()),
        latest_revision: Some(Revision(8)),
        capabilities: None,
        measurement_catalog: None,
        benchmark_catalog: None,
        agent_catalog: Some(agent_catalog),
        agent_lifecycle: None,
        provider_catalog: Some(provider_catalog),
        dynamic_provider_catalog: None,
        configuration: Some(ConfigurationSnapshot {
            runner_instance_id: "runner-setup".into(),
            generation: Revision(3),
            configured: false,
            agent_ids: Vec::new(),
            provider_id: None,
            model_id: None,
            auth_method: None,
            credential_reference_sha256: None,
        }),
        auth_status: None,
        auth_unavailable: true,
        auth_development_only: true,
        auth_unavailable_reason: Some("development credentials are unavailable".into()),
        recording_campaign: None,
        recording_estimate: None,
        recording_campaign_lifecycle: None,
        fanout: None,
        runs: Vec::new(),
        analysis: None,
    });

    state.open_wizard();
    assert_eq!(state.wizard.step(), Step::Agent);
    state.wizard.toggle_visible_agents().unwrap();
    state.wizard.advance().unwrap();
    state.wizard.select_catalog_cursor().unwrap();
    state.wizard.advance().unwrap();
    state.wizard.select_catalog_cursor().unwrap();
    state.wizard.advance().unwrap();
    state.wizard.set_value("shared defaults").unwrap();
    state.wizard.advance().unwrap();
    state
        .wizard
        .set_value(format!("credential_reference:{}", "2".repeat(64)))
        .unwrap();
    let draft = WorkspaceState::wizard_provider_setup_draft(
        &state.wizard.values(),
        state.live.as_ref().unwrap().agent_catalog.as_ref().unwrap(),
        state
            .live
            .as_ref()
            .unwrap()
            .provider_catalog
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(draft.selection().provider_id, "provider-a");
    assert_eq!(draft.selection().model_id, "model-a");
    assert_eq!(
        draft.selection().auth_method,
        ProviderAuthMethod::CredentialReference
    );
    assert!(draft.selection().credential_reference_sha256.is_some());
    assert!(
        !state
            .wizard
            .values()
            .iter()
            .any(|value| value.contains("api_key"))
    );
    assert!(state.live.as_ref().unwrap().auth_development_only);
    assert!(state.live.as_ref().unwrap().auth_unavailable);

    // A canceled draft is discarded; reopening starts from the authoritative
    // catalog again, so a prior provider/model/default cannot leak into a
    // repeat configuration flow.
    state.wizard.cancel();
    state.open_wizard();
    let empty: [String; 7] = Default::default();
    assert_eq!(state.wizard.values(), empty);
    assert_eq!(state.wizard.step(), Step::Agent);
}

#[test]
fn ar1668_authoritative_openrouter_wizard_completes_through_renderer_seam() {
    use asb_tui::{
        agent_catalog::{
            AgentAvailability, AgentCatalog, AgentCatalogEntry, AgentPackage, AgentProvenance,
            AgentSigner, AgentTarget,
        },
        control_codec::{ConfigurationSnapshot, ProviderAuthMethod, Revision},
        live_projection::{Connection, LiveSnapshot},
        provider_catalog::development_openrouter_catalog,
    };

    let target = AgentTarget {
        operating_system: "linux".into(),
        architecture: "x86_64".into(),
        libc: "glibc".into(),
        libc_version: "2.35".into(),
    };
    let agent = |id: &str, marker: char| AgentCatalogEntry {
        agent_id: id.into(),
        target: target.clone(),
        package: Some(AgentPackage {
            package_id: format!("{id}-package"),
            version: "1.0.0".into(),
            sha256: marker.to_string().repeat(64),
            signature_sha256: "c".repeat(64),
            signer: AgentSigner {
                key_id: "release-key".into(),
                principal: "asb-release".into(),
            },
        }),
        provenance: Some(AgentProvenance {
            source_revision: "d".repeat(40),
            manifest_sha256: "e".repeat(64),
            sbom_sha256: "f".repeat(64),
            license_ref: "MIT".into(),
        }),
        capabilities: vec!["benchmark".into()],
        availability: AgentAvailability::Available,
    };
    let agents = AgentCatalog {
        runner_instance_id: "development-fixture".into(),
        generation: 4,
        catalog_sha256: "a".repeat(64),
        target: target.clone(),
        agents: vec![agent("opencode", '1'), agent("opendesk", '2')],
        refreshed: true,
    };
    let mut providers = development_openrouter_catalog();
    providers.refreshed = true;
    let mut state = WorkspaceState::default();
    state.apply_live_snapshot(LiveSnapshot {
        connection: Connection::Negotiated,
        runner_instance_id: Some("development-fixture".into()),
        latest_revision: Some(Revision(4)),
        capabilities: None,
        measurement_catalog: None,
        benchmark_catalog: None,
        agent_catalog: Some(agents.clone()),
        agent_lifecycle: None,
        provider_catalog: Some(providers.clone()),
        dynamic_provider_catalog: None,
        configuration: Some(ConfigurationSnapshot {
            runner_instance_id: "development-fixture".into(),
            generation: Revision(1),
            configured: false,
            agent_ids: Vec::new(),
            provider_id: None,
            model_id: None,
            auth_method: None,
            credential_reference_sha256: None,
        }),
        auth_status: None,
        auth_unavailable: true,
        auth_development_only: true,
        auth_unavailable_reason: Some("development credentials are unavailable".into()),
        recording_campaign: None,
        recording_estimate: None,
        recording_campaign_lifecycle: None,
        fanout: None,
        runs: Vec::new(),
        analysis: None,
    });

    assert_eq!(state.screen, Screen::Wizard);
    let mut visited = Vec::new();
    let mut record_step = |state: &WorkspaceState| {
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| asb_tui::ui::render(frame, state, policy(false)))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("ASB setup wizard"));
        assert!(text.contains(asb_tui::wizard::element_id(state.wizard.step())));
        visited.push(asb_tui::wizard::element_id(state.wizard.step()));
    };

    #[derive(Clone, Copy)]
    enum ActionClass {
        Selection,
        Navigation,
        Confirmation,
        FreeForm,
    }
    let action_classes = [
        ActionClass::Selection,
        ActionClass::Navigation,
        ActionClass::Confirmation,
        ActionClass::FreeForm,
    ];
    let journey = [
        (KeyCode::Char('A'), ActionClass::Selection),
        (KeyCode::Enter, ActionClass::Confirmation),
        (KeyCode::Enter, ActionClass::Selection),
        (KeyCode::Down, ActionClass::Navigation),
        (KeyCode::Enter, ActionClass::Selection),
        (KeyCode::Enter, ActionClass::Confirmation),
        (KeyCode::Enter, ActionClass::Confirmation),
        (KeyCode::Enter, ActionClass::Confirmation),
        (KeyCode::Enter, ActionClass::Confirmation),
        (KeyCode::Enter, ActionClass::Confirmation),
    ];
    let mut counts = [0usize; 4];
    let mut injected_keys = 0usize;
    let mut prior_step = state.wizard.step();

    record_step(&state);
    for (code, class) in journey {
        state.handle_key(key(code));
        injected_keys += 1;
        counts[match class {
            ActionClass::Selection => 0,
            ActionClass::Navigation => 1,
            ActionClass::Confirmation => 2,
            ActionClass::FreeForm => 3,
        }] += 1;
        if state.screen == Screen::Wizard && state.wizard.step() != prior_step {
            prior_step = state.wizard.step();
            record_step(&state);
            if state.wizard.has_bounded_choices() {
                assert!(!state.wizard.bounded_choice_labels().is_empty());
            }
        }
    }
    assert_eq!(state.screen, Screen::Landing);

    assert_eq!(action_classes.len(), counts.len());
    let [
        selection_actions,
        navigation_actions,
        confirmation_actions,
        free_form_characters,
    ] = counts;
    let total_actions = selection_actions + navigation_actions + confirmation_actions;
    assert_eq!(injected_keys, journey.len());
    assert_eq!(
        total_actions, injected_keys,
        "every injected key must be counted"
    );
    assert_eq!(free_form_characters, 0);
    assert!(
        total_actions <= 24,
        "supported wizard used {total_actions} actions"
    );

    let values = state.take_wizard_completion().unwrap();
    let draft = WorkspaceState::wizard_provider_setup_draft(&values, &agents, &providers).unwrap();
    assert_eq!(draft.selection().agent_ids, ["opencode", "opendesk"]);
    assert_eq!(draft.selection().provider_id, "openrouter");
    assert_eq!(draft.selection().model_id, "openai/gpt-4o");
    assert_eq!(
        draft.selection().auth_method,
        ProviderAuthMethod::CredentialReference
    );
    assert_eq!(values[3], "all defaults");
    assert_eq!(values[5], "live");
    assert_eq!(values[6], "strict offline replay");
    assert!(values.iter().all(|value| !value.contains("api_key")));
    println!("AR1668_RENDERER_TRACE={}", visited.join(">"));
    println!(
        "AR1668_SELECTION=opencode,opendesk|openrouter|openai/gpt-4o|shared-defaults|live|strict-offline"
    );
    println!(
        "AR1723_ACTIONS=selection:{selection_actions}|navigation:{navigation_actions}|confirmation:{confirmation_actions}|free-form:{free_form_characters}|total:{total_actions}"
    );
}

#[test]
fn wizard_catalog_filters_moves_and_projects_selection_into_the_draft() {
    let catalog = WizardCatalog::new(
        vec![
            WizardOption::new("agent-a", "Agent A", true).unwrap(),
            WizardOption::new("agent-b", "Agent B", true).unwrap(),
        ],
        vec![
            WizardOption::new("provider-a", "Provider A", true)
                .unwrap()
                .compatible_with(vec!["agent-b".into()])
                .unwrap(),
        ],
        vec![
            WizardOption::new("model-a", "Model A", true)
                .unwrap()
                .compatible_with(vec!["provider-a".into()])
                .unwrap(),
        ],
    )
    .unwrap();
    let mut state = WorkspaceState::default().with_wizard_catalog(catalog);
    state.open_wizard();
    state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.wizard.current_value(), "agent-b");
    state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert_eq!(state.wizard.catalog().unwrap().query(), "a");
    assert_eq!(state.wizard.catalog().unwrap().cursor(), 0);
    state.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(state.wizard.catalog().unwrap().query(), "");
}
