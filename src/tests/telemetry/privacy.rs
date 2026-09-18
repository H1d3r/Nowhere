use super::*;
#[test]
fn aliases_are_instance_local_and_category_separated() {
    let p = Privacy::new().unwrap();
    let other = Privacy::new().unwrap();
    let value = "secret.example:443";
    assert_eq!(p.alias("target", value), p.alias("target", value));
    assert_ne!(p.alias("target", value), p.alias("client", value));
    assert_ne!(p.alias("target", value), other.alias("target", value));
    assert!(!p.alias("target", value).contains(value));
}
