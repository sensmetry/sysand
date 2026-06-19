// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::exceptions::{ExceptionKind, JniExt};
use indexmap::IndexMap;
use jni::{
    Env, jni_sig, jni_str,
    objects::{IntoAuto as _, JMap, JObject, JObjectArray, JString, JValue},
    signature::{FieldSignature, MethodSignature},
    strings::JNIStr,
};
use sysand_core::{
    build::{KParBuildError, KparCompressionMethod},
    model::{
        InterchangeProjectChecksum, InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw,
        InterchangeProjectMetadataRaw, InterchangeProjectUsageRaw,
    },
};
use sysand_core::{project::local_src::LocalSrcError, utils::format_err};

pub(crate) const STRING: FieldSignature = jni_sig!(java.lang.String);
pub(crate) const BOOLEAN: FieldSignature = jni_sig!(java.lang.Boolean);
pub(crate) const LINKED_HASH_MAP: FieldSignature = jni_sig!(java.util.LinkedHashMap);
pub(crate) const INTERCHANGE_PROJECT_USAGE_RESOURCE_CLASS: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProjectUsageResource);
pub(crate) const INTERCHANGE_PROJECT_USAGE_RESOURCE_CLASS_CONSTRUCTOR: MethodSignature =
    jni_sig!((java.lang.String, java.lang.String));
pub(crate) const INTERCHANGE_PROJECT_USAGE_DIRECTORY_CLASS: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProjectUsageDirectory);
pub(crate) const INTERCHANGE_PROJECT_USAGE_DIRECTORY_CLASS_CONSTRUCTOR: MethodSignature =
    jni_sig!((java.lang.String, java.lang.String, java.lang.String));
pub(crate) const INTERCHANGE_PROJECT_USAGE_KPAR_PATH_CLASS: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProjectUsageKparPath);
pub(crate) const INTERCHANGE_PROJECT_USAGE_KPAR_PATH_CLASS_CONSTRUCTOR: MethodSignature =
    jni_sig!((java.lang.String, java.lang.String, java.lang.String));
pub(crate) const INTERCHANGE_PROJECT_USAGE_CLASS: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProjectUsage);
pub(crate) const INTERCHANGE_PROJECT_USAGE_CLASS_ARRAY: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProjectUsage[]);
pub(crate) const INTERCHANGE_PROJECT_INFO_CLASS: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProjectInfo);
pub(crate) const INTERCHANGE_PROJECT_INFO_CLASS_CONSTRUCTOR: MethodSignature = jni_sig!((java.lang.String, java.lang.String, java.lang.String, java.lang.String, java.lang.String, java.lang.String[], java.lang.String, java.lang.String[], com.sensmetry.sysand.model.InterchangeProjectUsage[]));
pub(crate) const INTERCHANGE_PROJECT_METADATA_CLASS: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProjectMetadata);
pub(crate) const INTERCHANGE_PROJECT_METADATA_CLASS_CONSTRUCTOR: MethodSignature = jni_sig!((
    java.util.LinkedHashMap,
    java.lang.String,
    java.lang.String,
    java.lang.Boolean,
    java.lang.Boolean,
    java.util.LinkedHashMap
));
pub(crate) const INTERCHANGE_PROJECT_CLASS: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProject);
pub(crate) const INTERCHANGE_PROJECT_CLASS_CONSTRUCTOR: MethodSignature = jni_sig!((
    com.sensmetry.sysand.model.InterchangeProjectInfo,
    com.sensmetry.sysand.model.InterchangeProjectMetadata
));
pub(crate) const INTERCHANGE_PROJECT_CHECKSUM_CLASS: FieldSignature =
    jni_sig!(com.sensmetry.sysand.model.InterchangeProjectChecksum);
pub(crate) const INTERCHANGE_PROJECT_CHECKSUM_CLASS_CONSTRUCTOR: MethodSignature =
    jni_sig!((java.lang.String, java.lang.String));

/// Unwrap or throw a `RuntimeException` with the given format string
/// as a message
macro_rules! unwrap_throw {
    ($env:expr, $expr:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                $env.throw_runtime_exception(format!(
                    concat!($fmt, ": {} (at {}:{})"),
                    $($arg,)*
                    format_err(e), file!(), line!()
                ));
                return None;
            }
        }
    };
}

/// Get an object field's value as a `JObject`, still possibly null.
fn get_field_object<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'local>,
    field_name: &JNIStr,
    sig: FieldSignature,
) -> Option<JObject<'local>> {
    let v = unwrap_throw!(
        env,
        env.get_field(obj, field_name, sig),
        "Failed to get field `{}`",
        field_name
    );
    let field_obj = unwrap_throw!(env, v.l(), "Failed to get field `{}`", field_name);
    Some(field_obj)
}

// Returns None on JNI error, Some(None) for a null field, Some(Some(v)) for a
// non-null field converted by `extract`.
fn get_nullable_field<'local, T>(
    env: &mut Env<'local>,
    obj: &JObject<'local>,
    field_name: &JNIStr,
    sig: FieldSignature,
    extract: impl FnOnce(&mut Env<'local>, JObject<'local>) -> Option<T>,
) -> Option<Option<T>> {
    let field_obj = get_field_object(env, obj, field_name, sig)?;
    if field_obj.is_null() {
        return Some(None);
    }
    Some(Some(extract(env, field_obj)?))
}

fn cast_to_string<'local>(
    env: &mut Env<'local>,
    field_obj: JObject<'local>,
    field_name: &JNIStr,
) -> Option<String> {
    let jstr = unwrap_throw!(
        env,
        JString::cast_local(env, field_obj),
        "Failed to cast field `{}` to String",
        field_name
    );
    env.get_str(&jstr, field_name)
}

fn get_string_field<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'local>,
    field_name: &JNIStr,
) -> Option<String> {
    let field_obj = get_field_object(env, obj, field_name, STRING)?;
    cast_to_string(env, field_obj, field_name)
}

fn get_nullable_string_field<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'local>,
    field_name: &JNIStr,
) -> Option<Option<String>> {
    get_nullable_field(env, obj, field_name, STRING, |env, field_obj| {
        cast_to_string(env, field_obj, field_name)
    })
}

fn get_string_array_field<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'local>,
    field_name: &JNIStr,
) -> Option<Vec<String>> {
    let field_obj = get_field_object(env, obj, field_name, jni_sig!(java.lang.String[]))?;
    let arr = unwrap_throw!(
        env,
        JObjectArray::<JString>::cast_local(env, field_obj),
        "Failed to get field {}",
        field_name
    );
    let len = unwrap_throw!(
        env,
        arr.len(env),
        "Failed to get length of `{}`",
        field_name
    );
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let elem = unwrap_throw!(
            env,
            arr.get_element(env, i),
            "Failed to get element of `{}[{}]`",
            field_name,
            i
        );
        let s = unwrap_throw!(
            env,
            elem.mutf8_chars(env),
            "Failed to read string element of `{}[{}]`",
            field_name,
            i
        );
        result.push(s.into());
    }
    Some(result)
}

fn get_nullable_boolean_field<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'local>,
    field_name: &JNIStr,
) -> Option<Option<bool>> {
    get_nullable_field(env, obj, field_name, BOOLEAN, |env, field_obj| {
        let v = unwrap_throw!(
            env,
            env.call_method(
                &field_obj,
                jni_str!("booleanValue"),
                jni_sig!(() -> bool),
                &[],
            ),
            "Failed to call booleanValue on `{}`",
            field_name
        );
        let b = unwrap_throw!(env, v.z(), "Failed to unbox Boolean `{}`", field_name);
        Some(b)
    })
}

fn try_instance_of<'local>(
    env: &mut Env<'local>,
    elem: &JObject<'local>,
    class: FieldSignature,
    label: &str,
) -> Option<bool> {
    let is = unwrap_throw!(
        env,
        env.is_instance_of(elem, class.sig()),
        "Failed to check type of `{}`",
        label
    );
    Some(is)
}

fn get_usage_array_field<'local>(
    env: &mut Env<'local>,
    obj: &JObject<'local>,
) -> Option<Vec<InterchangeProjectUsageRaw>> {
    let arr_obj = get_field_object(
        env,
        obj,
        jni_str!("usage"),
        INTERCHANGE_PROJECT_USAGE_CLASS_ARRAY,
    )?;
    let arr = unwrap_throw!(
        env,
        JObjectArray::<JObject>::cast_local(env, arr_obj),
        "Failed to cast usages to array"
    );
    let len = unwrap_throw!(env, arr.len(env), "Failed to get length of `usage`",);
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let elem = unwrap_throw!(
            env,
            arr.get_element(env, i),
            "Failed to get element `usage[{}]`",
            i
        );
        let label = format!("usage[{i}]");
        if try_instance_of(env, &elem, INTERCHANGE_PROJECT_USAGE_RESOURCE_CLASS, &label)? {
            let resource = get_string_field(env, &elem, jni_str!("resource"))?;
            let version_constraint =
                get_nullable_string_field(env, &elem, jni_str!("versionConstraint"))?;
            result.push(InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint,
            });
        } else if try_instance_of(
            env,
            &elem,
            INTERCHANGE_PROJECT_USAGE_DIRECTORY_CLASS,
            &label,
        )? {
            let dir = get_string_field(env, &elem, jni_str!("directory"))?;
            let publisher = get_string_field(env, &elem, jni_str!("publisher"))?;
            let name = get_string_field(env, &elem, jni_str!("name"))?;
            result.push(InterchangeProjectUsageRaw::Directory {
                dir,
                publisher,
                name,
            });
        } else if try_instance_of(
            env,
            &elem,
            INTERCHANGE_PROJECT_USAGE_KPAR_PATH_CLASS,
            &label,
        )? {
            let kpar_path = get_string_field(env, &elem, jni_str!("kparPath"))?;
            let publisher = get_string_field(env, &elem, jni_str!("publisher"))?;
            let name = get_string_field(env, &elem, jni_str!("name"))?;
            result.push(InterchangeProjectUsageRaw::KparPath {
                kpar_path,
                publisher,
                name,
            });
        } else {
            env.throw_runtime_exception(format!("Unknown usage type for `{label}`"));
            return None;
        }
    }
    Some(result)
}

pub(crate) fn java_info_to_raw<'local>(
    env: &mut Env<'local>,
    info: &JObject<'local>,
) -> Option<InterchangeProjectInfoRaw> {
    let name = get_string_field(env, info, jni_str!("name"))?;
    let publisher = get_nullable_string_field(env, info, jni_str!("publisher"))?;
    let description = get_nullable_string_field(env, info, jni_str!("description"))?;
    let version = get_string_field(env, info, jni_str!("version"))?;
    let license = get_nullable_string_field(env, info, jni_str!("license"))?;
    let maintainer = get_string_array_field(env, info, jni_str!("maintainer"))?;
    let website = get_nullable_string_field(env, info, jni_str!("website"))?;
    let topic = get_string_array_field(env, info, jni_str!("topic"))?;
    let usage = get_usage_array_field(env, info)?;
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
    env: &mut Env<'local>,
    meta: &JObject<'local>,
) -> Option<InterchangeProjectMetadataRaw> {
    let index_obj = get_field_object(env, meta, jni_str!("index"), LINKED_HASH_MAP)?;
    let index = java_map_to_index_map(env, index_obj)?;
    let created = get_string_field(env, meta, jni_str!("created"))?;
    let metamodel = get_nullable_string_field(env, meta, jni_str!("metamodel"))?;
    let includes_derived = get_nullable_boolean_field(env, meta, jni_str!("includesDerived"))?;
    let includes_implied = get_nullable_boolean_field(env, meta, jni_str!("includesImplied"))?;
    let checksum_obj = get_field_object(env, meta, jni_str!("checksum"), LINKED_HASH_MAP)?;
    let checksum = if checksum_obj.is_null() {
        None
    } else {
        let jmap = unwrap_throw!(
            env,
            JMap::cast_local(env, checksum_obj),
            "Failed to wrap checksum map"
        );
        let mut iter = unwrap_throw!(env, jmap.iter(env), "Failed to iterate checksum map");
        let mut result = IndexMap::new();
        while let Some(entry) =
            unwrap_throw!(env, iter.next(env), "Failed to iterate checksum map entry")
        {
            // `iter.next` and `entry.key`/`entry.value` each create a new local
            // reference per call; wrap them so they're released at the end of
            // each iteration instead of accumulating for the whole map.
            let entry = entry.auto();
            let key = unwrap_throw!(
                env,
                entry.key(env),
                "Failed to get key of checksum map entry",
            );
            let key = unwrap_throw!(
                env,
                JString::cast_local(env, key),
                "Failed to cast checksum entry path to string"
            )
            .auto();
            let key_chars = unwrap_throw!(
                env,
                key.mutf8_chars(env),
                "Failed to get chars of checksum map entry path"
            );
            let key_str = key_chars.into();

            let value = unwrap_throw!(
                env,
                entry.value(env),
                "Failed to get value of checksum map entry",
            )
            .auto();
            let cs_value = get_string_field(env, &value, jni_str!("value"))?;
            let cs_algorithm = get_string_field(env, &value, jni_str!("algorithm"))?;

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

pub(crate) trait ToJObject {
    /// `None` return = exception thrown. Parent must return
    /// ASAP to not eat the exception. If another exception is
    /// thrown before returning to JVM, current one will be lost
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>>;
}

pub(crate) trait ToJObjectArray {
    /// `None` return = exception thrown. Parent must return
    /// ASAP to not eat the exception. If another exception is
    /// thrown before returning to JVM, current one will be lost
    fn to_jobject_array<'local>(&self, env: &mut Env<'local>) -> Option<JObjectArray<'local>>;
}

impl<T: ToJObject> ToJObject for &T {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        ToJObject::to_jobject(*self, env)
    }
}

pub(crate) trait ToJString {
    /// `None` return = exception thrown. Parent must return
    /// ASAP to not eat the exception. If another exception is
    /// thrown before returning to JVM, current one will be lost
    fn to_jstring<'local>(&self, env: &mut Env<'local>) -> Option<JString<'local>>;
}

pub(crate) trait ToJStringArray {
    /// `None` return = exception thrown. Parent must return
    /// ASAP to not eat the exception. If another exception is
    /// thrown before returning to JVM, current one will be lost
    fn to_jstring_array<'local>(
        &self,
        env: &mut Env<'local>,
    ) -> Option<JObjectArray<'local, JString<'local>>>;
}

impl ToJString for String {
    fn to_jstring<'local>(&self, env: &mut Env<'local>) -> Option<JString<'local>> {
        self.as_str().to_jstring(env)
    }
}

impl ToJString for str {
    fn to_jstring<'local>(&self, env: &mut Env<'local>) -> Option<JString<'local>> {
        let s = unwrap_throw!(env, env.new_string(self), "Failed to create String");
        Some(s)
    }
}

impl ToJObject for String {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        self.as_str().to_jobject(env)
    }
}

impl ToJObject for str {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let s = unwrap_throw!(env, env.new_string(self), "Failed to create String");
        Some(s.into())
    }
}

impl<T: ToJObject> ToJObject for Option<T> {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        match self {
            Some(v) => v.to_jobject(env),
            // `None` specifically indicates that exception was thrown
            // because of a failure. In general, having `Option<T>::None`
            // is not a failure, so return `null` instead
            None => Some(JObject::null()),
        }
    }
}

impl<T: ToJString> ToJString for Option<T> {
    fn to_jstring<'local>(&self, env: &mut Env<'local>) -> Option<JString<'local>> {
        match self {
            Some(v) => v.to_jstring(env),
            // `None` specifically indicates that exception was thrown
            // because of a failure. In general, having `Option<T>::None`
            // is not a failure, so return `null` instead
            None => Some(JString::null()),
        }
    }
}

impl ToJStringArray for [String] {
    fn to_jstring_array<'local>(
        &self,
        env: &mut Env<'local>,
    ) -> Option<JObjectArray<'local, JString<'local>>> {
        let array = unwrap_throw!(
            env,
            JObjectArray::<JString>::new(env, self.len(), JString::null()),
            "Failed to create String[]",
        );
        for (i, value) in self.iter().enumerate() {
            let value_object = value.to_jstring(env)?;
            unwrap_throw!(
                env,
                array.set_element(env, i, value_object),
                "Failed to set String[] element"
            );
        }
        Some(array)
    }
}

impl ToJObject for bool {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let obj = unwrap_throw!(
            env,
            env.new_object(BOOLEAN.sig(), jni_sig!((bool)), &[JValue::from(*self)],),
            "Failed to create Boolean"
        );
        Some(obj)
    }
}

impl<K: ToJObject, V: ToJObject> ToJObject for IndexMap<K, V> {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let mut map = unwrap_throw!(
            env,
            env.new_object(LINKED_HASH_MAP.sig(), jni_sig!(()), &[],),
            "Failed to create LinkedHashMap",
        );
        for (key, value) in self.iter() {
            let key_object = key.to_jobject(env)?;
            let value_object = value.to_jobject(env)?;
            unwrap_throw!(
                env,
                env.call_method(
                    &mut map,
                    jni_str!("put"),
                    jni_sig!((java.lang.Object, java.lang.Object) -> java.lang.Object),
                    &[JValue::from(&key_object), JValue::from(&value_object)]
                ),
                "Failed to call LinkedHashMap::put()"
            );
        }
        Some(map)
    }
}

impl ToJObject for InterchangeProjectChecksum {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let value = self.value.to_jstring(env)?;
        let algorithm = {
            let s: &str = self.algorithm.into();
            s.to_jstring(env)?
        };
        let obj = unwrap_throw!(
            env,
            env.new_object(
                INTERCHANGE_PROJECT_CHECKSUM_CLASS.sig(),
                INTERCHANGE_PROJECT_CHECKSUM_CLASS_CONSTRUCTOR,
                &[JValue::from(&value), JValue::from(&algorithm)],
            ),
            "Failed to create LinkedHashMap",
        );
        Some(obj)
    }
}

impl ToJObject for InterchangeProjectChecksumRaw {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let value = self.value.to_jstring(env)?;
        let algorithm = self.algorithm.to_jstring(env)?;
        let obj = unwrap_throw!(
            env,
            env.new_object(
                INTERCHANGE_PROJECT_CHECKSUM_CLASS.sig(),
                INTERCHANGE_PROJECT_CHECKSUM_CLASS_CONSTRUCTOR,
                &[JValue::from(&value), JValue::from(&algorithm)],
            ),
            "Failed to create InterchangeProjectChecksum",
        );
        Some(obj)
    }
}

impl ToJObject for InterchangeProjectUsageRaw {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let usage = match self {
            InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint,
            } => {
                let resource = resource.to_jstring(env)?;
                let version_constraint = version_constraint.to_jstring(env)?;
                unwrap_throw!(
                    env,
                    env.new_object(
                        INTERCHANGE_PROJECT_USAGE_RESOURCE_CLASS.sig(),
                        INTERCHANGE_PROJECT_USAGE_RESOURCE_CLASS_CONSTRUCTOR,
                        &[JValue::from(&resource), JValue::from(&version_constraint)],
                    ),
                    "Failed to create InterchangeProjectUsageResource"
                )
            }
            InterchangeProjectUsageRaw::Directory {
                dir,
                publisher,
                name,
            } => {
                let dir = dir.to_jstring(env)?;
                let publisher = publisher.to_jstring(env)?;
                let name = name.to_jstring(env)?;
                unwrap_throw!(
                    env,
                    env.new_object(
                        INTERCHANGE_PROJECT_USAGE_DIRECTORY_CLASS.sig(),
                        INTERCHANGE_PROJECT_USAGE_DIRECTORY_CLASS_CONSTRUCTOR,
                        &[
                            JValue::from(&dir),
                            JValue::from(&publisher),
                            JValue::from(&name),
                        ],
                    ),
                    "Failed to create InterchangeProjectUsageDirectory"
                )
            }
            InterchangeProjectUsageRaw::KparPath {
                kpar_path,
                publisher,
                name,
            } => {
                let kpar_path = kpar_path.to_jobject(env)?;
                let publisher = publisher.to_jobject(env)?;
                let name = name.to_jobject(env)?;
                unwrap_throw!(
                    env,
                    env.new_object(
                        INTERCHANGE_PROJECT_USAGE_KPAR_PATH_CLASS.sig(),
                        INTERCHANGE_PROJECT_USAGE_KPAR_PATH_CLASS_CONSTRUCTOR,
                        &[
                            JValue::from(&kpar_path),
                            JValue::from(&publisher),
                            JValue::from(&name),
                        ],
                    ),
                    "Failed to create InterchangeProjectUsageKparPath"
                )
            }
        };
        Some(usage)
    }
}

impl ToJObjectArray for Vec<InterchangeProjectUsageRaw> {
    fn to_jobject_array<'local>(&self, env: &mut Env<'local>) -> Option<JObjectArray<'local>> {
        let array = unwrap_throw!(
            env,
            env.new_object_array(
                self.len()
                    .try_into()
                    .expect("Failed to convert length to i32"),
                INTERCHANGE_PROJECT_USAGE_CLASS.sig(),
                JObject::null(),
            ),
            "Failed to create InterchangeProjectUsage[]"
        );
        for (i, value) in self.iter().enumerate() {
            let value_object = value.to_jobject(env)?;
            unwrap_throw!(
                env,
                array.set_element(env, i, value_object),
                "Failed to set InterchangeProjectUsage[] element"
            );
        }
        Some(array)
    }
}

impl ToJObject for Vec<InterchangeProjectUsageRaw> {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        self.to_jobject_array(env).map(|v| v.into())
    }
}

impl ToJObject for InterchangeProjectInfoRaw {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let name = self.name.to_jstring(env)?;
        let publisher = self.publisher.to_jstring(env)?;
        let description = self.description.to_jstring(env)?;
        let version = self.version.to_jstring(env)?;
        let license = self.license.to_jstring(env)?;
        let maintainer = self.maintainer.to_jstring_array(env)?;
        let website = self.website.to_jstring(env)?;
        let topic = self.topic.to_jstring_array(env)?;
        let usage = self.usage.to_jobject(env)?;
        let obj = unwrap_throw!(
            env,
            env.new_object(
                INTERCHANGE_PROJECT_INFO_CLASS.sig(),
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
            ),
            "Failed to create InterchangeProjectInfo"
        );
        Some(obj)
    }
}

impl ToJObject for InterchangeProjectMetadataRaw {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let index = self.index.to_jobject(env)?;
        let created = self.created.to_jstring(env)?;
        let metamodel = self.metamodel.to_jstring(env)?;
        let includes_derived = self.includes_derived.to_jobject(env)?;
        let includes_implied = self.includes_implied.to_jobject(env)?;
        let checksum = self.checksum.to_jobject(env)?;
        let obj = unwrap_throw!(
            env,
            env.new_object(
                INTERCHANGE_PROJECT_METADATA_CLASS.sig(),
                INTERCHANGE_PROJECT_METADATA_CLASS_CONSTRUCTOR,
                &[
                    JValue::from(&index),
                    JValue::from(&created),
                    JValue::from(&metamodel),
                    JValue::from(&includes_derived),
                    JValue::from(&includes_implied),
                    JValue::from(&checksum),
                ],
            ),
            "Failed to create InterchangeProjectMetadata"
        );
        Some(obj)
    }
}

/// Convert `metadata.index` to `IndexMap`
pub(crate) fn java_map_to_index_map<'local>(
    env: &mut Env<'local>,
    map_obj: JObject<'local>,
) -> Option<IndexMap<String, String>> {
    let jmap = unwrap_throw!(
        env,
        JMap::cast_local(env, map_obj),
        "Failed to cast `index` to `java.util.Map`"
    );
    let mut iter = unwrap_throw!(env, jmap.iter(env), "Failed to get an iterator of `index`");
    let mut result = IndexMap::new();
    while let Some(entry) = unwrap_throw!(
        env,
        iter.next(env),
        "Failed to get next element of `index` iterator"
    ) {
        // `iter.next` and `entry.key`/`entry.value` each create a new local
        // reference per call; wrap them so they're released at the end of
        // each iteration instead of accumulating for the whole map.
        let entry = entry.auto();
        let key = unwrap_throw!(env, entry.key(env), "Failed to get key of `index` entry");
        let value = unwrap_throw!(
            env,
            entry.value(env),
            "Failed to get value of `index` entry"
        );
        let key = unwrap_throw!(
            env,
            JString::cast_local(env, key),
            "Failed to cast key of `index` entry to String"
        )
        .auto();
        let key_str = unwrap_throw!(
            env,
            key.mutf8_chars(env),
            "Failed to get chars of `index` entry key"
        );
        let value = unwrap_throw!(
            env,
            JString::cast_local(env, value),
            "Failed to cast value of `index` entry to String"
        )
        .auto();
        let value_str = unwrap_throw!(
            env,
            value.mutf8_chars(env),
            "Failed to get chars of `index` entry value"
        );
        result.insert(key_str.into(), value_str.into());
    }
    Some(result)
}

impl ToJObject for (InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw) {
    fn to_jobject<'local>(&self, env: &mut Env<'local>) -> Option<JObject<'local>> {
        let (info, metadata) = self;
        let info_object = info.to_jobject(env)?;
        let metadata_object = metadata.to_jobject(env)?;
        let obj = unwrap_throw!(
            env,
            env.new_object(
                INTERCHANGE_PROJECT_CLASS.sig(),
                INTERCHANGE_PROJECT_CLASS_CONSTRUCTOR,
                &[JValue::from(&info_object), JValue::from(&metadata_object)],
            ),
            "Failed to create InterchangeProject"
        );
        Some(obj)
    }
}

pub(crate) fn handle_build_error(env: &mut Env<'_>, error: KParBuildError<LocalSrcError>) {
    let e = format_err(&error);
    match error {
        KParBuildError::ProjectRead(_) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Project read error: {e}"),
            );
        }
        KParBuildError::Io(_) => {
            env.throw_exception(ExceptionKind::SysandException, format!("IO error: {e}"));
        }
        KParBuildError::Validation { .. } => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Validation error: {e}"),
            );
        }
        KParBuildError::Extract(_) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Extract error: {e}"),
            );
        }
        KParBuildError::UnknownFormat(_) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Unknown format error: {e}"),
            );
        }
        KParBuildError::MissingInfo => {
            env.throw_exception(
                ExceptionKind::SysandException,
                "Missing project information",
            );
        }
        KParBuildError::MissingMeta => {
            env.throw_exception(ExceptionKind::SysandException, "Missing project metadata");
        }
        KParBuildError::MissingInfoMeta => {
            env.throw_exception(
                ExceptionKind::SysandException,
                "Missing project information and metadata",
            );
        }
        KParBuildError::Zip(_) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Zip write error: {e}"),
            );
        }
        KParBuildError::Serialize(..) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Project serialization error: {e}"),
            );
        }
        KParBuildError::WorkspaceRead(_) => {
            env.throw_exception(
                ExceptionKind::SysandException,
                format!("Workspace read error: {e}"),
            );
        }
        KParBuildError::PathUsage(_) => {
            env.throw_exception(ExceptionKind::SysandException, e);
        }
        KParBuildError::WorkspaceMetamodelConflict { .. } => {
            env.throw_exception(ExceptionKind::SysandException, e);
        }
        KParBuildError::MissingIndexSymbol(_, _) => {
            env.throw_exception(ExceptionKind::InvalidValue, e)
        }
    }
}

pub(crate) fn compression_from_java_string(
    env: &mut Env<'_>,
    compression: String,
) -> Option<KparCompressionMethod> {
    match KparCompressionMethod::try_from(compression) {
        Ok(compression) => Some(compression),
        Err(err) => {
            env.throw_exception(ExceptionKind::SysandException, format_err(err));
            None
        }
    }
}
