// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::exceptions::JniExt;
use indexmap::IndexMap;
use jni::{
    JNIEnv,
    objects::{JObject, JObjectArray, JValue},
};
use sysand_core::model::{
    InterchangeProjectChecksum, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
    InterchangeProjectUsageRaw,
};

pub(crate) const INTERCHANGE_PROJECT_USAGE_CLASS: &str =
    "com/sensmetry/sysand/model/InterchangeProjectUsage";
pub(crate) const INTERCHANGE_PROJECT_INFO_CLASS: &str =
    "com/sensmetry/sysand/model/InterchangeProjectInfo";
pub(crate) const INTERCHANGE_PROJECT_INFO_CLASS_CONSTRUCTOR: &str = "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;[Lcom/sensmetry/sysand/model/InterchangeProjectUsage;)V";
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
        let algorithm = self.algorithm.to_jobject(env)?;
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

impl ToJObject for InterchangeProjectUsageRaw {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObject<'local>> {
        let resource = self.resource.to_jobject(env)?;
        let version_constraint = self.version_constraint.to_jobject(env)?;
        match env.new_object(
            INTERCHANGE_PROJECT_USAGE_CLASS,
            "()V",
            &[JValue::from(&resource), JValue::from(&version_constraint)],
        ) {
            Ok(o) => Some(o),
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to create LinkedHashMap: {e}"));
                None
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

impl ToJObjectArray for Vec<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    fn to_jobject_array<'local>(&self, env: &mut JNIEnv<'local>) -> Option<JObjectArray<'local>> {
        let mut array = match env.new_object_array(
            self.len()
                .try_into()
                .expect("Failed to convert length to i32"),
            INTERCHANGE_PROJECT_CLASS,
            JObject::null(),
        ) {
            Ok(o) => o,
            Err(e) => {
                env.throw_runtime_exception(format!("Failed to create InterchangeProject[]: {e}"));
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
                        "Failed to set InterchangeProject[] element: {e}"
                    ));
                    return None;
                }
            }
        }
        Some(array)
    }
}
