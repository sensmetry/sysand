// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use indexmap::IndexMap;
use jni::{
    JNIEnv,
    objects::{JObject, JObjectArray, JValue},
};
use sysand_core::model::{
    InterchangeProjectChecksum, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
    InterchangeProjectUsageRaw,
};

pub(crate) const INTERCHANGE_PROJECT_USAGE_CLASS: &str = "org/sysand/model/InterchangeProjectUsage";
pub(crate) const INTERCHANGE_PROJECT_INFO_CLASS: &str = "org/sysand/model/InterchangeProjectInfo";
pub(crate) const INTERCHANGE_PROJECT_INFO_CLASS_CONSTRUCTOR: &str = "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;[Lorg/sysand/model/InterchangeProjectUsage;)V";
pub(crate) const INTERCHANGE_PROJECT_METADATA_CLASS: &str =
    "org/sysand/model/InterchangeProjectMetadata";
pub(crate) const INTERCHANGE_PROJECT_METADATA_CLASS_CONSTRUCTOR: &str = "(Ljava/util/LinkedHashMap;Ljava/lang/String;Ljava/lang/String;Ljava/lang/Boolean;Ljava/lang/Boolean;Ljava/util/LinkedHashMap;)V";
pub(crate) const INTERCHANGE_PROJECT_CLASS: &str = "org/sysand/model/InterchangeProject";
pub(crate) const INTERCHANGE_PROJECT_CLASS_CONSTRUCTOR: &str =
    "(Lorg/sysand/model/InterchangeProjectInfo;Lorg/sysand/model/InterchangeProjectMetadata;)V";
pub(crate) const INTERCHANGE_PROJECT_CHECKSUM_CLASS: &str =
    "org/sysand/model/InterchangeProjectChecksum";
pub(crate) const INTERCHANGE_PROJECT_CHECKSUM_CLASS_CONSTRUCTOR: &str =
    "(Ljava/lang/String;Ljava/lang/String;)V";

pub(crate) trait ToJObject {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local>;
}

pub(crate) trait ToJObjectArray {
    fn to_jobject_array<'local>(&self, env: &mut JNIEnv<'local>) -> JObjectArray<'local>;
}

impl<T: ToJObject> ToJObject for &T {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        ToJObject::to_jobject(*self, env)
    }
}

impl ToJObject for String {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        env.new_string(self)
            .expect("Failed to create String")
            .into()
    }
}

impl ToJObject for str {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        env.new_string(self)
            .expect("Failed to create String")
            .into()
    }
}

impl<T: ToJObject> ToJObject for Option<T> {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        self.as_ref()
            .map(|value| value.to_jobject(env))
            .unwrap_or_default()
    }
}

impl ToJObjectArray for [String] {
    fn to_jobject_array<'local>(&self, env: &mut JNIEnv<'local>) -> JObjectArray<'local> {
        let string_class = env
            .find_class("java/lang/String")
            .expect("Failed to find String class");
        let mut array = env
            .new_object_array(
                self.len()
                    .try_into()
                    .expect("Failed to convert length to i32"),
                string_class,
                JObject::null(),
            )
            .expect("Failed to create ObjectArray");
        for (i, value) in self.iter().enumerate() {
            let index: i32 = i.try_into().expect("Failed to convert index to i32");
            let value_object = value.to_jobject(env);
            env.set_object_array_element(&mut array, index, value_object)
                .expect("Failed to set array element");
        }
        array
    }
}

impl ToJObject for [String] {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        self.to_jobject_array(env).into()
    }
}

impl ToJObject for bool {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let boolean_class = env
            .find_class("java/lang/Boolean")
            .expect("Failed to find Boolean class");
        let boolean_value: jni::sys::jboolean = if *self { 1 } else { 0 };
        let boolean_object = env
            .new_object(boolean_class, "(Z)V", &[JValue::from(boolean_value)])
            .expect("Failed to create Boolean");
        boolean_object
    }
}

impl<K: ToJObject, V: ToJObject> ToJObject for IndexMap<K, V> {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let index_class = env
            .find_class("java/util/LinkedHashMap")
            .expect("Failed to find LinkedHashMap class");
        let mut map = env
            .new_object(index_class, "()V", &[])
            .expect("Failed to create LinkedHashMap");
        for (key, value) in self.iter() {
            let key_object = key.to_jobject(env);
            let value_object = value.to_jobject(env);
            env.call_method(
                &mut map,
                "put",
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[JValue::from(&key_object), JValue::from(&value_object)],
            )
            .expect("Failed to call put");
        }
        map
    }
}

impl ToJObject for InterchangeProjectChecksum {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let checksum_class = env
            .find_class(INTERCHANGE_PROJECT_CHECKSUM_CLASS)
            .expect("Failed to find InterchangeProjectChecksum class");
        let value = self.value.to_jobject(env);
        let algorithm = self.algorithm.to_jobject(env);
        env.new_object(
            checksum_class,
            INTERCHANGE_PROJECT_CHECKSUM_CLASS_CONSTRUCTOR,
            &[JValue::from(&value), JValue::from(&algorithm)],
        )
        .expect("Failed to create InterchangeProjectChecksum")
    }
}

impl ToJObject for InterchangeProjectUsageRaw {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let usage_class = env
            .find_class(INTERCHANGE_PROJECT_USAGE_CLASS)
            .expect("Failed to find InterchangeProjectUsage class");
        let resource = self.resource.to_jobject(env);
        let version_constraint = self.version_constraint.to_jobject(env);
        env.new_object(
            usage_class,
            "()V",
            &[JValue::from(&resource), JValue::from(&version_constraint)],
        )
        .expect("Failed to create InterchangeProjectUsage")
    }
}

impl ToJObjectArray for Vec<InterchangeProjectUsageRaw> {
    fn to_jobject_array<'local>(&self, env: &mut JNIEnv<'local>) -> JObjectArray<'local> {
        let usage_class = env
            .find_class(INTERCHANGE_PROJECT_USAGE_CLASS)
            .expect("Failed to find InterchangeProjectUsage class");
        let mut array = env
            .new_object_array(
                self.len()
                    .try_into()
                    .expect("Failed to convert length to i32"),
                usage_class,
                JObject::null(),
            )
            .expect("Failed to create ObjectArray");
        for (i, value) in self.iter().enumerate() {
            let index: i32 = i.try_into().expect("Failed to convert index to i32");
            let value_object = value.to_jobject(env);
            env.set_object_array_element(&mut array, index, value_object)
                .expect("Failed to set array element");
        }
        array
    }
}

impl ToJObject for Vec<InterchangeProjectUsageRaw> {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        self.to_jobject_array(env).into()
    }
}

impl ToJObject for InterchangeProjectInfoRaw {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let info_class = env
            .find_class(INTERCHANGE_PROJECT_INFO_CLASS)
            .expect("Failed to find InterchangeProjectInfo class");
        let name = self.name.to_jobject(env);
        let description = self.description.to_jobject(env);
        let version = self.version.to_jobject(env);
        let license = self.license.to_jobject(env);
        let maintainer = self.maintainer.to_jobject(env);
        let website = self.website.to_jobject(env);
        let topic = self.topic.to_jobject(env);
        let usage = self.usage.to_jobject(env);
        env.new_object(
            info_class,
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
        )
        .expect("Failed to create InterchangeProjectInfo")
    }
}

impl ToJObject for InterchangeProjectMetadataRaw {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let metadata_class = env
            .find_class(INTERCHANGE_PROJECT_METADATA_CLASS)
            .expect("Failed to find InterchangeProjectMetadata class");
        let index = self.index.to_jobject(env);
        let created = self.created.to_jobject(env);
        let metamodel = self.metamodel.to_jobject(env);
        let includes_derived = self.includes_derived.to_jobject(env);
        let includes_implied = self.includes_implied.to_jobject(env);
        let checksum = self.checksum.to_jobject(env);
        env.new_object(
            metadata_class,
            INTERCHANGE_PROJECT_METADATA_CLASS_CONSTRUCTOR,
            &[
                JValue::from(&index),
                JValue::from(&created),
                JValue::from(&metamodel),
                JValue::from(&includes_derived),
                JValue::from(&includes_implied),
                JValue::from(&checksum),
            ],
        )
        .expect("Failed to create InterchangeProjectMetadata")
    }
}

impl ToJObject for (InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw) {
    fn to_jobject<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let (info, metadata) = self;
        let info_object = info.to_jobject(env);
        let metadata_object = metadata.to_jobject(env);
        let project_class = env
            .find_class(INTERCHANGE_PROJECT_CLASS)
            .expect("Failed to find InterchangeProject class");
        env.new_object(
            project_class,
            INTERCHANGE_PROJECT_CLASS_CONSTRUCTOR,
            &[JValue::from(&info_object), JValue::from(&metadata_object)],
        )
        .expect("Failed to create InterchangeProject")
    }
}

impl ToJObjectArray for Vec<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    fn to_jobject_array<'local>(&self, env: &mut JNIEnv<'local>) -> JObjectArray<'local> {
        let project_class = env
            .find_class(INTERCHANGE_PROJECT_CLASS)
            .expect("Failed to find InterchangeProject class");
        let mut array = env
            .new_object_array(
                self.len()
                    .try_into()
                    .expect("Failed to convert length to i32"),
                project_class,
                JObject::null(),
            )
            .expect("Failed to create ObjectArray");
        for (i, value) in self.iter().enumerate() {
            let index: i32 = i.try_into().expect("Failed to convert index to i32");
            let value_object = value.to_jobject(env);
            env.set_object_array_element(&mut array, index, value_object)
                .expect("Failed to set array element");
        }
        array
    }
}
