use crate::project::{local_kpar::LocalKParProject, reference::ProjectReference};
#[test]
fn test_kpar() {
    let kpar = ProjectReference::new(LocalKParProject::new("path", "root").unwrap());
    let _clone = kpar.clone();
}
