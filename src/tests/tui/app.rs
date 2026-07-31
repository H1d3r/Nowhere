use super::*;

#[test]
fn capability_detection_is_well_formed() {
    let capabilities = terminal_capabilities();
    let _ = (capabilities.color, capabilities.unicode);
}
