use work_core::{error::NameError, naming};

#[test]
fn names_follow_convention() {
    assert_eq!(naming::volume("acme"), "work-acme-home");
    assert_eq!(naming::network("acme"), "work-net-acme");
    assert_eq!(naming::container("acme"), "work-acme");
}

#[test]
fn valid_names_accepted() {
    assert!(naming::validate_name("a").is_ok());
    assert!(naming::validate_name("acme-1").is_ok());
    assert!(naming::validate_name("shopvision").is_ok());
}

#[test]
fn invalid_names_rejected() {
    assert_eq!(naming::validate_name(""), Err(NameError::Empty));
    assert_eq!(naming::validate_name("Acme"), Err(NameError::InvalidChar));
    assert_eq!(
        naming::validate_name("acme_dev"),
        Err(NameError::InvalidChar)
    );
    assert_eq!(naming::validate_name("-acme"), Err(NameError::InvalidChar));
    assert_eq!(
        naming::validate_name(&"a".repeat(41)),
        Err(NameError::TooLong)
    );
    assert_eq!(naming::validate_name("new"), Err(NameError::Reserved));
    assert_eq!(naming::validate_name("doctor"), Err(NameError::Reserved));
}
