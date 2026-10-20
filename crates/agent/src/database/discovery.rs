// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Finding database workloads among a cluster's pods.
//!
//! Pure functions over the pod list, so every rule can be tested without a
//! cluster: which pods are databases, which workload each belongs to, and which
//! of a workload's pods a dump should run in.

use std::{collections::BTreeMap, str::FromStr};

use k8s_openapi::api::core::v1::Pod;
use shared::database::{
    ANNOTATION_CONTAINER, ANNOTATION_DATABASE, ANNOTATION_DATABASES, ANNOTATION_PASSWORD_ENV,
    ANNOTATION_USER, DEFAULT_POSTGRES_USER, DatabaseEngine, DatabaseRuntime, DatabaseSourceId,
    DatabaseSourceKind,
};

use super::{commands::Client, exec::PodTarget, target::DumpTarget};

/// Label `CloudNativePG` puts on every pod of a cluster, naming the cluster.
pub const CNPG_CLUSTER_LABEL: &str = "cnpg.io/cluster";

/// Label `CloudNativePG` uses for an instance's current role.
const CNPG_ROLE_LABEL: &str = "cnpg.io/instanceRole";

/// The container `CloudNativePG` runs `PostgreSQL` in.
const CNPG_CONTAINER: &str = "postgres";

/// Label Kubernetes adds to the pods of a Deployment's `ReplicaSet`, whose
/// value is also the suffix of the `ReplicaSet`'s name.
const POD_TEMPLATE_HASH_LABEL: &str = "pod-template-hash";

/// A database workload, with where a dump of it would run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCandidate {
    pub id: DatabaseSourceId,
    pub kind: DatabaseSourceKind,
    pub engine: DatabaseEngine,
    pub target: DumpTarget,
    pub user: String,
    /// The variable in the container's environment holding the password,
    /// `None` for the ones the engine's image is set up with.
    pub password_env: Option<String>,
    /// The databases the annotation or label names, when it names any.
    /// `None` dumps every database that accepts connections.
    pub databases: Option<Vec<String>>,
}

impl SourceCandidate {
    /// The commands this source's server takes.
    #[must_use]
    pub fn client(&self) -> Client<'_> {
        Client::new(
            self.engine,
            &self.user,
            self.password_env.as_deref(),
            &self.target,
        )
    }
}

/// The databases a comma-separated annotation or label value names.
pub fn database_list(value: Option<&str>) -> Option<Vec<String>> {
    value.map(|list| {
        list.split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn pod_target(namespace: &str, pod: String, container: String) -> DumpTarget {
    DumpTarget::Pod(PodTarget {
        namespace: namespace.to_owned(),
        pod,
        container,
    })
}

/// A pod's lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::EnumString)]
enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

/// A `CloudNativePG` instance's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::EnumString)]
#[strum(serialize_all = "lowercase")]
enum CnpgRole {
    Primary,
    Replica,
}

/// The kind of object that owns a pod.
#[derive(Debug, Clone, PartialEq, Eq, strum_macros::EnumString)]
enum OwnerKind {
    ReplicaSet,
    #[strum(default)]
    Other(String),
}

/// The database workloads among `pods`, one per workload, ordered by
/// namespace and name.
pub fn find_sources(pods: &[Pod]) -> Vec<SourceCandidate> {
    let mut cnpg: BTreeMap<DatabaseSourceId, Vec<&Pod>> = BTreeMap::new();
    let mut annotated: BTreeMap<DatabaseSourceId, SourceCandidate> = BTreeMap::new();

    let mut running: Vec<&Pod> = pods.iter().filter(|pod| is_running(pod)).collect();
    running.sort_by(|a, b| {
        (&a.metadata.namespace, &a.metadata.name).cmp(&(&b.metadata.namespace, &b.metadata.name))
    });

    for pod in running {
        if let Some(id) = cnpg_cluster(pod) {
            cnpg.entry(id).or_default().push(pod);
        } else if let Some(candidate) = annotated_source(pod) {
            annotated.entry(candidate.id.clone()).or_insert(candidate);
        }
    }

    let mut sources: Vec<SourceCandidate> = cnpg
        .into_iter()
        .filter_map(|(id, pods)| cnpg_source(id, &pods))
        .chain(annotated.into_values())
        .collect();
    sources.sort_by(|a, b| a.id.cmp(&b.id));
    sources
}

fn is_running(pod: &Pod) -> bool {
    pod.metadata.deletion_timestamp.is_none()
        && pod
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
            .and_then(|phase| PodPhase::from_str(phase).ok())
            == Some(PodPhase::Running)
}

fn label<'a>(pod: &'a Pod, key: &str) -> Option<&'a str> {
    pod.metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get(key))
        .map(String::as_str)
}

fn annotation<'a>(pod: &'a Pod, key: &str) -> Option<&'a str> {
    pod.metadata
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.get(key))
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn cnpg_cluster(pod: &Pod) -> Option<DatabaseSourceId> {
    Some(DatabaseSourceId {
        runtime: DatabaseRuntime::Kubernetes,
        namespace: pod.metadata.namespace.clone()?,
        name: label(pod, CNPG_CLUSTER_LABEL)?.to_owned(),
    })
}

/// Dumps run on a replica when the cluster has one, so a nightly dump does
/// not load the primary. Pods arrive sorted, so the choice is stable.
fn cnpg_source(id: DatabaseSourceId, pods: &[&Pod]) -> Option<SourceCandidate> {
    let role =
        |pod: &&&Pod| label(pod, CNPG_ROLE_LABEL).and_then(|role| CnpgRole::from_str(role).ok());
    let chosen = pods
        .iter()
        .find(|pod| role(pod) == Some(CnpgRole::Replica))
        .or_else(|| pods.iter().find(|pod| role(pod) == Some(CnpgRole::Primary)))
        .or_else(|| pods.first())?;
    let target = pod_target(
        &id.namespace,
        chosen.metadata.name.clone()?,
        CNPG_CONTAINER.to_owned(),
    );
    Some(SourceCandidate {
        id,
        kind: DatabaseSourceKind::CnpgCluster,
        engine: DatabaseEngine::Postgresql,
        target,
        user: DEFAULT_POSTGRES_USER.to_owned(),
        password_env: None,
        databases: None,
    })
}

fn annotated_source(pod: &Pod) -> Option<SourceCandidate> {
    let engine = DatabaseEngine::from_str(annotation(pod, ANNOTATION_DATABASE)?).ok()?;
    let container = annotation(pod, ANNOTATION_CONTAINER)
        .map(str::to_owned)
        .or_else(|| {
            pod.spec
                .as_ref()
                .and_then(|spec| spec.containers.first())
                .map(|container| container.name.clone())
        })?;
    let databases = database_list(annotation(pod, ANNOTATION_DATABASES));
    let pod_name = pod.metadata.name.clone()?;
    let namespace = pod.metadata.namespace.clone()?;
    Some(SourceCandidate {
        target: pod_target(&namespace, pod_name.clone(), container),
        id: DatabaseSourceId {
            runtime: DatabaseRuntime::Kubernetes,
            namespace,
            name: workload_name(pod).unwrap_or_else(|| pod_name.clone()),
        },
        kind: DatabaseSourceKind::Annotated,
        engine,
        user: annotation(pod, ANNOTATION_USER)
            .unwrap_or(engine.default_user())
            .to_owned(),
        password_env: annotation(pod, ANNOTATION_PASSWORD_ENV).map(str::to_owned),
        databases,
    })
}

/// The workload a pod belongs to, which outlives the pod: the name of its
/// controlling owner, with a Deployment's `ReplicaSet` hash taken off so a
/// rollout does not turn the database into a new source.
fn workload_name(pod: &Pod) -> Option<String> {
    let owner = pod
        .metadata
        .owner_references
        .as_ref()?
        .iter()
        .find(|owner| owner.controller == Some(true))?;
    match OwnerKind::from_str(&owner.kind) {
        Ok(OwnerKind::ReplicaSet) => Some(
            label(pod, POD_TEMPLATE_HASH_LABEL)
                .and_then(|hash| owner.name.strip_suffix(&format!("-{hash}")))
                .unwrap_or(&owner.name)
                .to_owned(),
        ),
        Ok(OwnerKind::Other(_)) | Err(_) => Some(owner.name.clone()),
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::{
        api::core::v1::{Container, PodSpec, PodStatus},
        apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference, Time},
    };

    use super::*;

    fn pod(namespace: &str, name: &str) -> Pod {
        Pod {
            metadata: ObjectMeta {
                namespace: Some(namespace.to_owned()),
                name: Some(name.to_owned()),
                ..ObjectMeta::default()
            },
            spec: Some(PodSpec {
                containers: vec![
                    Container {
                        name: "db".to_owned(),
                        ..Container::default()
                    },
                    Container {
                        name: "metrics".to_owned(),
                        ..Container::default()
                    },
                ],
                ..PodSpec::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_owned()),
                ..PodStatus::default()
            }),
        }
    }

    fn with_labels(mut pod: Pod, labels: &[(&str, &str)]) -> Pod {
        pod.metadata.labels = Some(
            labels
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        );
        pod
    }

    fn with_annotations(mut pod: Pod, annotations: &[(&str, &str)]) -> Pod {
        pod.metadata.annotations = Some(
            annotations
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        );
        pod
    }

    fn owned_by(mut pod: Pod, kind: &str, name: &str) -> Pod {
        pod.metadata.owner_references = Some(vec![OwnerReference {
            kind: kind.to_owned(),
            name: name.to_owned(),
            controller: Some(true),
            ..OwnerReference::default()
        }]);
        pod
    }

    fn cnpg_pod(name: &str, role: &str) -> Pod {
        with_labels(
            pod("apps", name),
            &[(CNPG_CLUSTER_LABEL, "pg"), (CNPG_ROLE_LABEL, role)],
        )
    }

    #[test]
    fn a_cnpg_cluster_is_dumped_from_a_replica() {
        let sources = find_sources(&[
            cnpg_pod("pg-1", "primary"),
            cnpg_pod("pg-2", "replica"),
            cnpg_pod("pg-3", "replica"),
        ]);
        assert_eq!(sources.len(), 1);
        let source = sources.first().unwrap();
        assert_eq!(source.id.to_string(), "apps/pg");
        assert_eq!(source.kind, DatabaseSourceKind::CnpgCluster);
        assert_eq!(source.target.place(), "pg-2");
        assert_eq!(source.target.detail(), "postgres");
        assert_eq!(source.user, "postgres");
        assert_eq!(source.databases, None);
    }

    #[test]
    fn a_cnpg_cluster_without_replicas_is_dumped_from_its_primary() {
        let sources = find_sources(&[cnpg_pod("pg-1", "primary")]);
        assert_eq!(sources.first().unwrap().target.place(), "pg-1");

        let unlabelled = with_labels(pod("apps", "pg-9"), &[(CNPG_CLUSTER_LABEL, "pg")]);
        assert_eq!(
            find_sources(&[unlabelled]).first().unwrap().target.place(),
            "pg-9"
        );
    }

    #[test]
    fn pods_that_are_not_running_are_ignored() {
        let mut pending = cnpg_pod("pg-2", "replica");
        if let Some(status) = pending.status.as_mut() {
            status.phase = Some("Pending".to_owned());
        }
        let mut leaving = cnpg_pod("pg-3", "replica");
        leaving.metadata.deletion_timestamp = Some(Time(k8s_openapi::jiff::Timestamp::default()));
        let sources = find_sources(&[cnpg_pod("pg-1", "primary"), pending, leaving]);
        assert_eq!(sources.first().unwrap().target.place(), "pg-1");

        let mut no_status = cnpg_pod("pg-1", "primary");
        no_status.status = None;
        assert_eq!(find_sources(&[no_status]).len(), 0);
    }

    #[test]
    fn an_annotated_pod_reads_its_settings_from_its_annotations() {
        let annotated = owned_by(
            with_annotations(
                pod("shop", "db-0"),
                &[
                    (ANNOTATION_DATABASE, "postgresql"),
                    (ANNOTATION_CONTAINER, "metrics"),
                    (ANNOTATION_USER, "shop"),
                    (ANNOTATION_DATABASES, "orders, , users"),
                ],
            ),
            "StatefulSet",
            "db",
        );
        let sources = find_sources(&[annotated]);
        let source = sources.first().unwrap();
        assert_eq!(source.id.to_string(), "shop/db");
        assert_eq!(source.kind, DatabaseSourceKind::Annotated);
        assert_eq!(source.target.detail(), "metrics");
        assert_eq!(source.user, "shop");
        assert_eq!(
            source.databases,
            Some(vec!["orders".to_owned(), "users".to_owned()])
        );
    }

    #[test]
    fn an_annotated_pod_falls_back_to_its_first_container_and_postgres() {
        let sources = find_sources(&[with_annotations(
            pod("shop", "lonely"),
            &[(ANNOTATION_DATABASE, "postgresql"), (ANNOTATION_USER, "  ")],
        )]);
        let source = sources.first().unwrap();
        assert_eq!(source.id.name, "lonely");
        assert_eq!(source.target.detail(), "db");
        assert_eq!(source.user, "postgres");
    }

    #[test]
    fn a_deployments_replicas_are_one_source_named_after_it() {
        let replica = |name: &str| {
            owned_by(
                with_labels(
                    with_annotations(pod("web", name), &[(ANNOTATION_DATABASE, "postgresql")]),
                    &[(POD_TEMPLATE_HASH_LABEL, "7d9f")],
                ),
                "ReplicaSet",
                "pg-7d9f",
            )
        };
        let sources = find_sources(&[replica("pg-7d9f-b"), replica("pg-7d9f-a")]);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources.first().unwrap().id.name, "pg");
        assert_eq!(sources.first().unwrap().target.place(), "pg-7d9f-a");
    }

    #[test]
    fn unknown_engines_and_plain_pods_are_not_sources() {
        let sources = find_sources(&[
            with_annotations(pod("a", "oracle"), &[(ANNOTATION_DATABASE, "oracle")]),
            pod("a", "web"),
        ]);
        assert_eq!(sources.len(), 0);
    }

    #[test]
    fn sources_are_ordered_by_namespace_and_name() {
        let annotated = |ns: &str, name: &str| {
            with_annotations(pod(ns, name), &[(ANNOTATION_DATABASE, "postgresql")])
        };
        let names: Vec<String> = find_sources(&[
            annotated("b", "z"),
            annotated("a", "y"),
            cnpg_pod("pg-1", "primary"),
        ])
        .into_iter()
        .map(|source| source.id.to_string())
        .collect();
        assert_eq!(names, ["a/y", "apps/pg", "b/z"]);
    }
}
