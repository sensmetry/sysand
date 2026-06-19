// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::exceptions::JniExt;
use indexmap::IndexMap;
use jni::{
    JNIEnv,
    objects::{JMap, JObject, JObjectArray, JString, JValue},
};
use sysand_core::model::{
    InterchangeProjectChecksum, InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw,
    InterchangeProjectMetadataRaw, InterchangeProjectUsageRaw,
};

fn get_string_field<'local>(
    env: &mut JNIEnv<'local>,
    obj: &JObject<'local>,
    field_name: &str,
) -> Option<String> {
    let field_obj = match env.get_field(obj, field_name, "Ljava/lang/String;") {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
                return None;
            }
        },
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
            return None;
        }
    };
    match env.get_string(&JString::from(field_obj)) {
        Ok(s) => Some(s.into()),
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to read string field `{field_name}`: {e}"));
            None
        }
    }
}

// Returns None on JNI error, Some(None) for null, Some(Some(s)) for a value.
fn get_nullable_string_field<'local>(
    env: &mut JNIEnv<'local>,
    obj: &JObject<'local>,
    field_name: &str,
) -> Option<Option<String>> {
    let field_obj = match env.get_field(obj, field_name, "Ljava/lang/String;") {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
                return None;
            }
        },
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
            return None;
        }
    };
    if field_obj.is_null() {
        return Some(None);
    }
    match env.get_string(&JString::from(field_obj)) {
        Ok(s) => Some(Some(s.into())),
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to read string field `{field_name}`: {e}"));
            None
        }
    }
}

fn get_string_array_field<'local>(
    env: &mut JNIEnv<'local>,
    obj: &JObject<'local>,
    field_name: &str,
) -> Option<Vec<String>> {
    let arr_obj = match env.get_field(obj, field_name, "[Ljava/lang/String;") {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
                return None;
            }
        },
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
            return None;
        }
    };
    let arr = JObjectArray::from(arr_obj);
    let len = match env.get_array_length(&arr) {
        Ok(l) => l as usize,
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get length of `{field_name}`: {e}"));
            return None;
        }
    };
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let elem = match env.get_object_array_element(&arr, i as i32) {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!(
                    "Failed to get element of `{field_name}[{i}]`: {e}"
                ));
                return None;
            }
        };
        let s = match env.get_string(&JString::from(elem)) {
            Ok(s) => s.into(),
            Err(e) => {
                env.throw_runtime_exception(format!(
                    "Failed to read string element of `{field_name}[{i}]`: {e}"
                ));
                return None;
            }
        };
        result.push(s);
    }
    Some(result)
}

// Returns None on JNI error, Some(None) for null, Some(Some(b)) for a value.
fn get_nullable_boolean_field<'local>(
    env: &mut JNIEnv<'local>,
    obj: &JObject<'local>,
    field_name: &str,
) -> Option<Option<bool>> {
    let field_obj = match env.get_field(obj, field_name, "Ljava/lang/Boolean;") {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
                return None;
            }
        },
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
            return None;
        }
    };
    if field_obj.is_null() {
        return Some(None);
    }
    match env.call_method(&field_obj, "booleanValue", "()Z", &[]) {
        Ok(v) => match v.z() {
            Ok(b) => Some(Some(b)),
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to unbox Boolean `{field_name}`: {e}"));
                None
            }
        },
        Err(e) => {
            env.throw_runtime_exception(format!(
                "Failed to call booleanValue on `{field_name}`: {e}"
            ));
            None
        }
    }
}

fn get_usage_array_field<'local>(
    env: &mut JNIEnv<'local>,
    obj: &JObject<'local>,
    field_name: &str,
) -> Option<Vec<InterchangeProjectUsageRaw>> {
    let sig = format!("[L{INTERCHANGE_PROJECT_USAGE_CLASS};");
    let arr_obj = match env.get_field(obj, field_name, sig.as_str()) {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
                return None;
            }
        },
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get field `{field_name}`: {e}"));
            return None;
        }
    };
    let arr = JObjectArray::from(arr_obj);
    let len = match env.get_array_length(&arr) {
        Ok(l) => l as usize,
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get length of `{field_name}`: {e}"));
            return None;
        }
    };
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let elem = match env.get_object_array_element(&arr, i as i32) {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!(
                    "Failed to get element of `{field_name}[{i}]`: {e}"
                ));
                return None;
            }
        };
        match env.is_instance_of(&elem, INTERCHANGE_PROJECT_USAGE_RESOURCE_CLASS) {
            Ok(true) => {
                let resource = get_string_field(env, &elem, "resource")?;
                let version_constraint =
                    get_nullable_string_field(env, &elem, "versionConstraint")?;
                result.push(InterchangeProjectUsageRaw::Resource {
                    resource,
                    version_constraint,
                });
            }
            Ok(false) => {
                env.throw_runtime_exception(
                    "Unknown usage type, only InterchangeProjectUsageResource is supported",
                );
                return None;
            }
            Err(e) => {
                env.throw_runtime_exception(format!(
                    "Failed to check whether `{field_name}[{i}]` is InterchangeProjectUsageResource:\n\
                    {e}"
                ));
                return None;
            }
        }
    }
    Some(result)
}

pub(crate) fn java_info_to_raw<'local>(
    env: &mut JNIEnv<'local>,
    info: &JObject<'local>,
) -> Option<InterchangeProjectInfoRaw> {
    let name = get_string_field(env, info, "name")?;
    let publisher = get_nullable_string_field(env, info, "publisher")?;
    let description = get_nullable_string_field(env, info, "description")?;
    let version = get_string_field(env, info, "version")?;
    let license = get_nullable_string_field(env, info, "license")?;
    let maintainer = get_string_array_field(env, info, "maintainer")?;
    let website = get_nullable_string_field(env, info, "website")?;
    let topic = get_string_array_field(env, info, "topic")?;
    let usage = get_usage_array_field(env, info, "usage")?;
    Some(InterchangeProjectInfoRaw {
        name,
        publisher,
        description,
        version,
        license,
        maintainer,
        website,
        topic,
        usage,
    })
}

pub(crate) fn java_metadata_to_raw<'local>(
    env: &mut JNIEnv<'local>,
    meta: &JObject<'local>,
) -> Option<InterchangeProjectMetadataRaw> {
    let index_obj = match env.get_field(meta, "index", "Ljava/util/LinkedHashMap;") {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to get field `index`: {e}"));
                return None;
            }
        },
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get field `index`: {e}"));
            return None;
        }
    };
    let index = match java_map_to_index_map(env, &index_obj) {
        Ok(m) => m,
        Err(jni::errors::Error::JavaException) => return None,
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to convert index map: {e}"));
            return None;
        }
    };
    let created = get_string_field(env, meta, "created")?;
    let metamodel = get_nullable_string_field(env, meta, "metamodel")?;
    let includes_derived = get_nullable_boolean_field(env, meta, "includesDerived")?;
    let includes_implied = get_nullable_boolean_field(env, meta, "includesImplied")?;
    let checksum_obj = match env.get_field(meta, "checksum", "Ljava/util/LinkedHashMap;") {
        Ok(v) => match v.l() {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to get field `checksum`: {e}"));
                return None;
            }
        },
        Err(e) => {
            env.throw_runtime_exception(format!("Failed to get field `checksum`: {e}"));
            return None;
        }
    };
    let checksum = if checksum_obj.is_null() {
        None
    } else {
        let jmap = match JMap::from_env(env, &checksum_obj) {
            Ok(m) => m,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to wrap checksum map: {e}"));
                return None;
            }
        };
        let mut iter = match jmap.iter(env) {
            Ok(i) => i,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to iterate checksum map: {e}"));
                return None;
            }
        };
        let mut result = IndexMap::new();
        loop {
            let entry = match iter.next(env) {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => {
                    env.throw_runtime_exception(format!(
                        "Failed to iterate checksum map entry: {e}"
                    ));
                    return None;
                }
            };
            let (key, value) = entry;
            let key_str: String = match env.get_string(&JString::from(key)) {
                Ok(s) => s.into(),
                Err(e) => {
                    env.throw_runtime_exception(format!("Failed to read checksum key: {e}"));
                    return None;
                }
            };
            let cs_value = get_string_field(env, &value, "value")?;
            let cs_algorithm = get_string_field(env, &value, "algorithm")?;
            result.insert(
                key_str,
                InterchangeProjectChecksumRaw {
                    value: cs_value,
                    algorithm: cs_algorithm,
                },
            );
        }
        Some(result)
    };
    Some(InterchangeProjectMetadataRaw {
        index,
        created,
        metamodel,
        includes_derived,
        includes_implied,
        checksum,
    })
}

pub(crate) const INTERCHANGE_PROJECT_USAGE_RESOURCE_CLASS: &str =
    "com/sensmetry/sysand/model/InterchangeProjectUsageResource";
pub(crate) const INTERCHANGE_PROJECT_USAGE_DIRECTORY_CLASS: &str =
    "com/sensmetry/sysand/model/InterchangeProjectUsageDirectory";
pub(crate) const INTERCHANGE_PROJECT_USAGE_CLASS: &str =
    "com/sensmetry/sysand/model/InterchangeProjectUsage";
pub(crate) const INTERCHANGE_PROJECT_INFO_CLASS: &str =
    "com/sensmetry/sysand/model/InterchangeProjectInfo";
pub(crate) const INTERCHANGE_PROJECT_INFO_CLASS_CONSTRUCTOR: &str = "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;[Lcom/sensmetry/sysand/model/InterchangeProjectUsage;)V";
pub(crate) const INTERCHANGE_PROJECT_METADATA_CLASS: &str =
    "com/sensmetry/sysand/model/InterchangeProjectMetadata";
pub(crate) const INTERCHANGE_PROJECT_METADATA_CLASS_CONSTRUCTOR: &str = "(Ljava/util/LinkedHashMap;Ljava/lang/String;Ljava/lang/String;Ljava/lang/Boolean;Ljava/lang/Boolean;Ljava/util/LinkedHashMap;)V";
pub(crate) const INTERCHANGE_PROJECT_CLASS: &str = "com/sensmetry/sysand/model/InterchangeProject";
pub(crate) const INTERCHANGE_PROJECT_CLASS_CONSTRUCTOR: &str = "(Lcom/sensmetry/sysand/model/InterchangeProjectInfo;Lcom/sensmetry/sysand/model/InterchangeProjectMetadata;)V";
pub(crate) const INTERCHANGE_PROJECT_CHECKSUM_CLASS: &str =
    "com/sensmetry/sysand/model/InterchangeProjectChecksum";
pub(crate) const INTERCHANGE_PROJECT_CHECKSUM_CLASS_CONSTRUCTOR: &str =
    "(Ljava/lang/String;Ljava/lang/String;)V";

pub(crate) trait ToJObject {
    /// `None` return = exception thrown. Parent must return
    /// ASAP to not eat the exception. If another exception is
    /// thrown before returning to JVM, current one will be lost
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>>;
}

pub(crate) trait ToJObjectArray {
    /// `None` return = exception thrown. Parent must return
    /// ASAP to not eat the exception. If another exception is
    /// thrown before returning to JVM, current one will be lost
    fn to_jobject_array<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObjectArray<'local>>;
}

impl<T: ToJObject> ToJObject for &T {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        ToJObject::to_jobject(*self, env)
    }
}

impl ToJObject for String {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        self.as_str().to_jobject(env)
    }
}

impl ToJObject for str {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        match env.new_string(self) {
            Ok(s) => Some(s.into()),
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to create String: {e}"));
                None
            }
        }
    }
}

impl<T: ToJObject> ToJObject for Option<T> {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        match self {
            Some(v) => v.to_jobject(env),
            // `None` specifically indicates that exception was thrown
            // because of a failure. In general, having `Option<T>::None`
            // is not a failure, so return `null` instead
            None => Some(JObject::null()),
        }
    }
}

impl ToJObjectArray for [String] {
    fn to_jobject_array<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObjectArray<'local>> {
        let mut array = match env.new_object_array(
            self.len()
                .try_into()
                .expect("Failed to convert length to i32"),
            "java/lang/String",
            JObject::null(),
        ) {
            Ok(a) => a,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to create String[]: {e}"));
                return None;
            }
        };
        for (i, value) in self.iter().enumerate() {
            let index: i32 = i.try_into().expect("Failed to convert index to i32");
            let value_object = value.to_jobject(env)?;
            match env.set_object_array_element(&mut array, index, value_object) {
                Ok(_) => (),
                Err(e) => {
                    env.throw_runtime_exception(format!("Failed to set String[] element: {e}"));
                    return None;
                }
            };
        }
        Some(array)
    }
}

impl ToJObject for [String] {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        Some(self.to_jobject_array(env)?.into())
    }
}

impl ToJObject for bool {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        let boolean_value: jni::sys::jboolean = if *self { 1 } else { 0 };
        match env.new_object("java/lang/Boolean", "(Z)V", &[JValue::from(boolean_value)]) {
            Ok(b) => Some(b),
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to create Boolean: {e}"));
                None
            }
        }
    }
}

impl<K: ToJObject, V: ToJObject> ToJObject for IndexMap<K, V> {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        let mut map = match env.new_object("java/util/LinkedHashMap", "()V", &[]) {
            Ok(l) => l,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to create LinkedHashMap: {e}"));
                return None;
            }
        };
        for (key, value) in self.iter() {
            let key_object = key.to_jobject(env)?;
            let value_object = value.to_jobject(env)?;
            match env.call_method(
                &mut map,
                "put",
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[JValue::from(&key_object), JValue::from(&value_object)],
            ) {
                Ok(_) => (),
                Err(e) => {
                    env.throw_runtime_exception(format!(
                        "Failed to call LinkedHashMap::put(): {e}"
                    ));
                    return None;
                }
            }
        }
        Some(map)
    }
}

impl ToJObject for InterchangeProjectChecksum {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        let value = self.value.to_jobject(env)?;
        let algorithm = {
            let s: &str = self.algorithm.into();
            s.to_jobject(env)?
        };
        match env.new_object(
            INTERCHANGE_PROJECT_CHECKSUM_CLASS,
            INTERCHANGE_PROJECT_CHECKSUM_CLASS_CONSTRUCTOR,
            &[JValue::from(&value), JValue::from(&algorithm)],
        ) {
            Ok(o) => Some(o),
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to create LinkedHashMap: {e}"));
                None
            }
        }
    }
}

impl ToJObject for InterchangeProjectChecksumRaw {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        let value = self.value.to_jobject(env)?;
        let algorithm = self.algorithm.to_jobject(env)?;
        match env.new_object(
            INTERCHANGE_PROJECT_CHECKSUM_CLASS,
            INTERCHANGE_PROJECT_CHECKSUM_CLASS_CONSTRUCTOR,
            &[JValue::from(&value), JValue::from(&algorithm)],
        ) {
            Ok(o) => Some(o),
            Err(e) => {
                env.throw_runtime_exception(format!(
                    "Failed to create InterchangeProjectChecksum: {e}"
                ));
                None
            }
        }
    }
}

impl ToJObject for InterchangeProjectUsageRaw {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        match self {
            InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint,
            } => {
                let resource = resource.to_jobject(env)?;
                let version_constraint = version_constraint.to_jobject(env)?;
                match env.new_object(
                    INTERCHANGE_PROJECT_USAGE_RESOURCE_CLASS,
                    "(Ljava/lang/String;Ljava/lang/String;)V",
                    &[JValue::from(&resource), JValue::from(&version_constraint)],
                ) {
                    Ok(o) => Some(o),
                    Err(e) => {
                        env.throw_runtime_exception(format!(
                            "Failed to create InterchangeProjectUsageResource: {e}"
                        ));
                        None
                    }
                }
            }
            InterchangeProjectUsageRaw::Directory {
                dir,
                publisher,
                name,
            } => {
                let dir = dir.to_jobject(env)?;
                let publisher = publisher.to_jobject(env)?;
                let name = name.to_jobject(env)?;
                match env.new_object(
                    INTERCHANGE_PROJECT_USAGE_DIRECTORY_CLASS,
                    "(Ljava/lang/String;Ljava/lang/String;)V",
                    &[
                        JValue::from(&dir),
                        JValue::from(&publisher),
                        JValue::from(&name),
                    ],
                ) {
                    Ok(o) => Some(o),
                    Err(e) => {
                        env.throw_runtime_exception(format!(
                            "Failed to create InterchangeProjectUsageDirectory: {e}"
                        ));
                        None
                    }
                }
            }
        }
    }
}

impl ToJObjectArray for Vec<InterchangeProjectUsageRaw> {
    fn to_jobject_array<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObjectArray<'local>> {
        let mut array = match env.new_object_array(
            self.len()
                .try_into()
                .expect("Failed to convert length to i32"),
            INTERCHANGE_PROJECT_USAGE_CLASS,
            JObject::null(),
        ) {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!(
                    "Failed to create InterchangeProjectUsage[]: {e}"
                ));
                return None;
            }
        };
        for (i, value) in self.iter().enumerate() {
            let index: i32 = i.try_into().expect("Failed to convert index to i32");
            let value_object = value.to_jobject(env)?;
            match env.set_object_array_element(&mut array, index, value_object) {
                Ok(o) => o,
                Err(e) => {
                    env.throw_runtime_exception(format!(
                        "Failed to set InterchangeProjectUsage[] element: {e}"
                    ));
                    return None;
                }
            }
        }
        Some(array)
    }
}

impl ToJObject for Vec<InterchangeProjectUsageRaw> {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        self.to_jobject_array(env).map(|v| v.into())
    }
}

impl ToJObject for InterchangeProjectInfoRaw {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        let name = self.name.to_jobject(env)?;
        let publisher = self.publisher.to_jobject(env)?;
        let description = self.description.to_jobject(env)?;
        let version = self.version.to_jobject(env)?;
        let license = self.license.to_jobject(env)?;
        let maintainer = self.maintainer.to_jobject(env)?;
        let website = self.website.to_jobject(env)?;
        let topic = self.topic.to_jobject(env)?;
        let usage = self.usage.to_jobject(env)?;
        match env.new_object(
            INTERCHANGE_PROJECT_INFO_CLASS,
            INTERCHANGE_PROJECT_INFO_CLASS_CONSTRUCTOR,
            &[
                JValue::from(&name),
                JValue::from(&publisher),
                JValue::from(&description),
                JValue::from(&version),
                JValue::from(&license),
                JValue::from(&maintainer),
                JValue::from(&website),
                JValue::from(&topic),
                JValue::from(&usage),
            ],
        ) {
            Ok(o) => Some(o),
            Err(e) => {
                env.throw_runtime_exception(format!(
                    "Failed to create InterchangeProjectInfo: {e}"
                ));
                None
            }
        }
    }
}

impl ToJObject for InterchangeProjectMetadataRaw {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        let index = self.index.to_jobject(env)?;
        let created = self.created.to_jobject(env)?;
        let metamodel = self.metamodel.to_jobject(env)?;
        let includes_derived = self.includes_derived.to_jobject(env)?;
        let includes_implied = self.includes_implied.to_jobject(env)?;
        let checksum = self.checksum.to_jobject(env)?;
        match env.new_object(
            INTERCHANGE_PROJECT_METADATA_CLASS,
            INTERCHANGE_PROJECT_METADATA_CLASS_CONSTRUCTOR,
            &[
                JValue::from(&index),
                JValue::from(&created),
                JValue::from(&metamodel),
                JValue::from(&includes_derived),
                JValue::from(&includes_implied),
                JValue::from(&checksum),
            ],
        ) {
            Ok(o) => Some(o),
            Err(e) => {
                env.throw_runtime_exception(format!(
                    "Failed to create InterchangeProjectMetadata: {e}"
                ));
                None
            }
        }
    }
}

pub(crate) fn java_map_to_index_map<'local>(
    env: &mut JNIEnv<'local>,
    map_obj: &JObject<'local>,
) -> Result<IndexMap<String, String>, jni::errors::Error> {
    let jmap = JMap::from_env(env, map_obj)?;
    let mut iter = jmap.iter(env)?;
    let mut result = IndexMap::new();
    while let Some((key, value)) = iter.next(env)? {
        let key_str = env
            .get_str(&JString::from(key), "index map key")
            .ok_or(jni::errors::Error::JavaException)?;
        let value_str = env
            .get_str(&JString::from(value), "index map value")
            .ok_or(jni::errors::Error::JavaException)?;
        result.insert(key_str, value_str);
    }
    Ok(result)
}

impl ToJObject for (InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw) {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        let (info, metadata) = self;
        let info_object = info.to_jobject(env)?;
        let metadata_object = metadata.to_jobject(env)?;
        match env.new_object(
            INTERCHANGE_PROJECT_CLASS,
            INTERCHANGE_PROJECT_CLASS_CONSTRUCTOR,
            &[JValue::from(&info_object), JValue::from(&metadata_object)],
        ) {
            Ok(o) => Some(o),
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to create InterchangeProject: {e}"));
                None
            }
        }
    }
}
