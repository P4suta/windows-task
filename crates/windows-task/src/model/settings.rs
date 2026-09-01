#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{TaskDuration, TaskLimit};

/// Behavior when a previous instance is still running.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum MultipleInstancesPolicy {
    /// Starts the next instance in parallel.
    Parallel,
    /// Enqueues one pending instance.
    Queue,
    /// Does not start another instance.
    #[default]
    IgnoreNew,
    /// Stops the existing instance before starting the new one.
    StopExisting,
}

/// Automatic restart behavior after failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct RestartPolicy {
    /// Delay between restart attempts.
    pub interval: TaskDuration,
    /// Maximum number of attempts.
    pub count: u8,
}

/// Idle-condition behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct IdleSettings {
    /// Required idle duration.
    pub duration: TaskDuration,
    /// Maximum time to wait for the idle condition.
    pub wait_timeout: TaskDuration,
    /// Stops the task when the computer stops being idle.
    pub stop_on_idle_end: bool,
    /// Restarts the task when idle resumes.
    pub restart_on_idle: bool,
}

impl Default for IdleSettings {
    fn default() -> Self {
        Self {
            duration: TaskDuration::from_secs(10 * 60),
            wait_timeout: TaskDuration::from_secs(60 * 60),
            stop_on_idle_end: true,
            restart_on_idle: false,
        }
    }
}

/// Network profile required for task start.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct NetworkSettings {
    /// Network profile GUID as text.
    pub id: Option<String>,
    /// Human-readable profile name.
    pub name: Option<String>,
}

/// Automatic maintenance settings introduced by newer scheduler versions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct MaintenanceSettings {
    /// How often the maintenance task should run.
    pub period: TaskDuration,
    /// Deadline after the period begins.
    pub deadline: TaskDuration,
    /// Allows Windows to run outside automatic maintenance when needed.
    pub exclusive: bool,
}

/// Complete Task Scheduler settings model for the 2.0 schema family.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct TaskSettings {
    /// Allows explicit Run/RunEx calls.
    pub allow_demand_start: bool,
    /// Failure restart policy.
    pub restart_on_failure: Option<RestartPolicy>,
    /// Concurrent-instance policy.
    pub multiple_instances: MultipleInstancesPolicy,
    /// Prevents start while on battery power.
    pub disallow_start_if_on_batteries: bool,
    /// Stops a running task when battery power begins.
    pub stop_if_going_on_batteries: bool,
    /// Allows Task Scheduler to terminate the task forcibly.
    pub allow_hard_terminate: bool,
    /// Runs a missed time-based task as soon as possible.
    pub start_when_available: bool,
    /// Starts only when a selected network is available.
    pub network: Option<NetworkSettings>,
    /// Per-task execution time limit.
    pub execution_time_limit: TaskLimit,
    /// Whether the task can fire.
    pub enabled: bool,
    /// Removes an expired task after this delay.
    pub delete_expired_after: Option<TaskDuration>,
    /// Scheduler priority from 0 (highest) through 10 (lowest).
    pub priority: u8,
    /// Idle requirements; `None` means no idle condition.
    pub idle: Option<IdleSettings>,
    /// Wakes a sleeping computer to run the task.
    pub wake_to_run: bool,
    /// Hides the task from default enumeration.
    pub hidden: bool,
    /// Uses the unified scheduling engine.
    pub use_unified_scheduling_engine: bool,
    /// Prevents start in RemoteApp sessions.
    pub disallow_start_on_remote_app_session: bool,
    /// Automatic maintenance policy.
    pub maintenance: Option<MaintenanceSettings>,
    /// Deletes the task when its registration scope ends.
    pub volatile: bool,
}

impl Default for TaskSettings {
    fn default() -> Self {
        Self {
            allow_demand_start: true,
            restart_on_failure: None,
            multiple_instances: MultipleInstancesPolicy::IgnoreNew,
            disallow_start_if_on_batteries: true,
            stop_if_going_on_batteries: true,
            allow_hard_terminate: true,
            start_when_available: false,
            network: None,
            execution_time_limit: TaskLimit::default(),
            enabled: true,
            delete_expired_after: None,
            priority: 7,
            idle: None,
            wake_to_run: false,
            hidden: false,
            use_unified_scheduling_engine: false,
            disallow_start_on_remote_app_session: false,
            maintenance: None,
            volatile: false,
        }
    }
}
