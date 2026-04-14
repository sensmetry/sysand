use crate::auth::{GlobMapBuilder, GlobMapResultMut};

#[test]
fn basic_globmap_lookup() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = GlobMapBuilder::new();
    builder.add("a*.com/*", 1);
    builder.add("a*.com/**", 2);
    builder.add("b.com/*", 3);
    builder.add("a*.com/*/*", 4);
    let mut globmap = builder.build()?;

    if let GlobMapResultMut::Ambiguous(vals) = globmap.lookup_mut("axx.com/xxx") {
        let vals: Vec<i32> = vals.into_iter().map(|(_, i)| *i).collect();
        assert_eq!(vals, vec![1, 2]);
    } else {
        panic!("Expected ambiguous result.");
    }

    if let GlobMapResultMut::Ambiguous(vals) = globmap.lookup_mut("axx.com/xxx/xxx") {
        let vals: Vec<i32> = vals.into_iter().map(|(_, i)| *i).collect();
        assert_eq!(vals, vec![2, 4]);
    } else {
        panic!("Expected ambiguous result.");
    }

    let key = "axx.com/xxx/xxx/xxx";
    if let GlobMapResultMut::Found(k, val) = globmap.lookup_mut(key) {
        assert_eq!(k, key);
        assert_eq!(*val, 2);
    } else {
        panic!("Expected unambiguous result.");
    }

    let key = "b.com/xxx";
    if let GlobMapResultMut::Found(k, val) = globmap.lookup_mut(key) {
        assert_eq!(k, key);
        assert_eq!(*val, 3);
    } else {
        panic!("Expected unambiguous result.");
    }

    if let GlobMapResultMut::NotFound = globmap.lookup_mut("axx.com") {
    } else {
        panic!("Expected no result.");
    }

    if let GlobMapResultMut::NotFound = globmap.lookup_mut("bxx.com/xxx") {
    } else {
        panic!("Expected no result.");
    }

    if let GlobMapResultMut::NotFound = globmap.lookup_mut("cxx.com/xxx") {
    } else {
        panic!("Expected no result.");
    }

    Ok(())
}
