use crate::model::{
    DriverDiff, RegChange, RegSnapshot, RegValue, RegistryDiff, ServiceChange, ServiceDiff,
    ServiceInfo, TaskDiff, TaskSnapshot,
};
use std::collections::BTreeMap;

const SVC_PREFIX: &str = "HKLM\\SYSTEM\\CurrentControlSet\\Services\\";

pub fn registry(before: &RegSnapshot, after: &RegSnapshot) -> RegistryDiff {
    let mut d = RegistryDiff::default();

    for k in after.keys() {
        if !before.contains_key(k) {
            d.created_keys.push(k.clone());
        }
    }

    for (k, vals) in after {
        match before.get(k) {
            None => {
                for (name, (kind, data)) in vals {
                    d.created_values.push(RegValue {
                        key: k.clone(),
                        name: name.clone(),
                        kind: *kind,
                        data: data.clone(),
                    });
                }
            }
            Some(old) => {
                for (name, (kind, data)) in vals {
                    match old.get(name) {
                        None => d.created_values.push(RegValue {
                            key: k.clone(),
                            name: name.clone(),
                            kind: *kind,
                            data: data.clone(),
                        }),
                        Some((_, odata)) => {
                            if odata != data {
                                d.modified_values.push(RegChange {
                                    key: k.clone(),
                                    name: name.clone(),
                                    kind: *kind,
                                    old: odata.clone(),
                                    new: data.clone(),
                                });
                            }
                        }
                    }
                }
                for (name, (kind, data)) in old {
                    if !vals.contains_key(name) {
                        d.deleted_values.push(RegValue {
                            key: k.clone(),
                            name: name.clone(),
                            kind: *kind,
                            data: data.clone(),
                        });
                    }
                }
            }
        }
    }

    d
}

fn start_name(v: &str) -> String {
    match v {
        "0" => "boot".into(),
        "1" => "system".into(),
        "2" => "auto".into(),
        "3" => "manual".into(),
        "4" => "disabled".into(),
        other => other.into(),
    }
}

fn service_entries(snap: &RegSnapshot, drivers: bool) -> BTreeMap<String, ServiceInfo> {
    let mut out = BTreeMap::new();
    for (k, vals) in snap {
        let rest = match k.strip_prefix(SVC_PREFIX) {
            Some(r) => r,
            None => continue,
        };
        if rest.contains('\\') {
            continue;
        }
        let ty = vals
            .get("Type")
            .map(|(_, d)| d.parse::<u32>().unwrap_or(0))
            .unwrap_or(0);
        if ty == 0 {
            continue;
        }
        let is_driver = ty & 0x1 != 0;
        if is_driver != drivers {
            continue;
        }
        out.insert(
            rest.to_string(),
            ServiceInfo {
                name: rest.to_string(),
                display: vals
                    .get("DisplayName")
                    .map(|(_, d)| d.clone())
                    .unwrap_or_default(),
                image: vals
                    .get("ImagePath")
                    .map(|(_, d)| d.clone())
                    .unwrap_or_default(),
                start: start_name(
                    vals.get("Start")
                        .map(|(_, d)| d.as_str())
                        .unwrap_or_default(),
                ),
            },
        );
    }
    out
}

pub fn services(before: &RegSnapshot, after: &RegSnapshot) -> ServiceDiff {
    let b = service_entries(before, false);
    let a = service_entries(after, false);
    let mut d = ServiceDiff::default();

    for (name, info) in &a {
        match b.get(name) {
            None => d.created.push(info.clone()),
            Some(old) => {
                for (field, o, n) in [
                    ("image", &old.image, &info.image),
                    ("start", &old.start, &info.start),
                    ("display", &old.display, &info.display),
                ] {
                    if o != n {
                        d.modified.push(ServiceChange {
                            name: name.clone(),
                            field: field.to_string(),
                            old: o.clone(),
                            new: n.clone(),
                        });
                    }
                }
            }
        }
    }

    d
}

pub fn drivers(before: &RegSnapshot, after: &RegSnapshot) -> DriverDiff {
    let b = service_entries(before, true);
    let a = service_entries(after, true);
    DriverDiff {
        created: a
            .into_iter()
            .filter(|(k, _)| !b.contains_key(k))
            .map(|(_, v)| v)
            .collect(),
    }
}

pub fn tasks(before: &TaskSnapshot, after: &TaskSnapshot) -> TaskDiff {
    TaskDiff {
        created: after
            .keys()
            .filter(|k| !before.contains_key(*k))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RegKind;

    fn snap(entries: &[(&str, &[(&str, &str)])]) -> RegSnapshot {
        let mut m = BTreeMap::new();
        for (k, vals) in entries {
            let mut vm = BTreeMap::new();
            for (n, v) in *vals {
                vm.insert(n.to_string(), (RegKind::Sz, v.to_string()));
            }
            m.insert(k.to_string(), vm);
        }
        m
    }

    #[test]
    fn detects_created_key_and_values() {
        let b = snap(&[]);
        let a = snap(&[("HKCU\\SOFTWARE\\X", &[("A", "1")])]);
        let d = registry(&b, &a);
        assert_eq!(d.created_keys, vec!["HKCU\\SOFTWARE\\X"]);
        assert_eq!(d.created_values.len(), 1);
    }

    #[test]
    fn detects_modified_value() {
        let b = snap(&[("HKCU\\SOFTWARE\\X", &[("A", "1")])]);
        let a = snap(&[("HKCU\\SOFTWARE\\X", &[("A", "2")])]);
        let d = registry(&b, &a);
        assert_eq!(d.modified_values.len(), 1);
        assert_eq!(d.modified_values[0].old, "1");
        assert_eq!(d.modified_values[0].new, "2");
    }

    #[test]
    fn detects_deleted_value() {
        let b = snap(&[("HKCU\\SOFTWARE\\X", &[("A", "1")])]);
        let a = snap(&[("HKCU\\SOFTWARE\\X", &[])]);
        let d = registry(&b, &a);
        assert_eq!(d.deleted_values.len(), 1);
    }

    fn svc(name: &str, ty: &str) -> (String, BTreeMap<String, (RegKind, String)>) {
        let mut v = BTreeMap::new();
        v.insert("Type".to_string(), (RegKind::Dword, ty.to_string()));
        v.insert(
            "ImagePath".to_string(),
            (RegKind::Sz, "C:\\x.exe".to_string()),
        );
        v.insert("Start".to_string(), (RegKind::Dword, "3".to_string()));
        (format!("{}{}", SVC_PREFIX, name), v)
    }

    #[test]
    fn separates_services_from_drivers() {
        let mut b = BTreeMap::new();
        let mut a = BTreeMap::new();
        b.insert(SVC_PREFIX.to_string(), BTreeMap::new());
        a.insert(SVC_PREFIX.to_string(), BTreeMap::new());
        let (k1, v1) = svc("SvcA", "16");
        let (k2, v2) = svc("DrvB", "1");
        a.insert(k1, v1);
        a.insert(k2, v2);

        let s = services(&b, &a);
        assert_eq!(s.created.len(), 1);
        assert_eq!(s.created[0].name, "SvcA");

        let d = drivers(&b, &a);
        assert_eq!(d.created.len(), 1);
        assert_eq!(d.created[0].name, "DrvB");
    }

    #[test]
    fn tasks_only_reports_new_files() {
        let mut b = TaskSnapshot::new();
        b.insert("Old".into(), (1, 1));
        let mut a = b.clone();
        a.insert("New".into(), (2, 2));
        let d = tasks(&b, &a);
        assert_eq!(d.created, vec!["New"]);
    }
}
