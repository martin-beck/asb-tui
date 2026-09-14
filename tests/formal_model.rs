use asb_tui::formal_model::UiModel;

#[test]
fn published_model_covers_every_route_and_serializes() {
    let model = UiModel::current();
    model.validate().expect("formal TUI model must be closed");
    assert_eq!(model.routes.len(), 7);
    assert!(model.elements.len() >= 16);
    assert!(model.transitions.len() >= 18);
    assert!(model.to_json().unwrap().contains("measurement_selection"));
}
