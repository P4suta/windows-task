use super::{from_bytes, to_string, to_utf16le};
use crate::model::*;

fn round_trip(definition: &TaskDefinition) {
    let xml = to_string(definition).expect("serialize schema fixture");
    assert_eq!(
        from_bytes(xml.as_bytes()).expect("parse UTF8 fixture"),
        *definition
    );
    assert_eq!(
        from_bytes(&to_utf16le(definition).expect("serialize UTF16 fixture"))
            .expect("parse UTF16 fixture"),
        *definition
    );
}

#[test]
fn inactive_native_idle_settings_do_not_enable_idle_only_execution() {
    for condition in ["", "<RunOnlyIfIdle>false</RunOnlyIfIdle>"] {
        let input = format!(
            r#"<Task version="1.6"><Settings>{condition}<IdleSettings><StopOnIdleEnd>true</StopOnIdleEnd></IdleSettings></Settings><Actions><Exec><Command>fixture.exe</Command></Exec></Actions></Task>"#
        );
        let definition = from_bytes(input.as_bytes()).expect("native idle defaults");
        assert!(definition.settings.idle.is_none());
        assert!(
            to_string(&definition)
                .expect("write")
                .contains("<RunOnlyIfIdle>false</RunOnlyIfIdle>")
        );
    }
}

#[test]
fn all_known_actions_preserve_fields() {
    for action in [
        Action::Exec(ExecAction {
            id: Some("exec".into()),
            command: "fixture.exe".into(),
            arguments: Some("&<quoted>".into()),
            working_directory: Some("C:\\fixture".into()),
            hide_window: true,
        }),
        Action::ComHandler(ComHandlerAction {
            id: Some("com".into()),
            class_id: uuid::Uuid::nil(),
            data: Some("&<data>".into()),
        }),
        Action::Email(EmailAction {
            id: Some("email".into()),
            server: "localhost".into(),
            subject: Some("subject".into()),
            from: Some("from@example.test".into()),
            to: Some("to@example.test".into()),
            cc: Some("cc@example.test".into()),
            bcc: Some("bcc@example.test".into()),
            reply_to: Some("reply@example.test".into()),
            body: Some("&<body>".into()),
            attachments: vec!["C:\\fixture.txt".into()],
            headers: vec![EmailHeader {
                name: "X-Fixture".into(),
                value: "value".into(),
            }],
        }),
        Action::ShowMessage(ShowMessageAction {
            id: Some("message".into()),
            title: Some("title".into()),
            body: "&<message>".into(),
        }),
    ] {
        round_trip(&TaskDefinition::new(action));
    }
}

#[test]
fn all_known_trigger_families_preserve_common_and_specific_fields() {
    let delay = Some(TaskDuration::from_secs(60));
    let common = TriggerCommon {
        id: Some("trigger".into()),
        start_boundary: Some("2026-01-01T01:00:00+09:00".parse().expect("date")),
        end_boundary: Some("2027-01-01T01:00:00+09:00".parse().expect("date")),
        enabled: true,
        execution_time_limit: Some(TaskLimit::Unlimited),
        repetition: Some(Repetition {
            interval: TaskDuration::from_secs(60),
            duration: Some(TaskDuration::from_secs(3600)),
            stop_at_duration_end: true,
        }),
    };
    let triggers = vec![
        Trigger::Boot(BootTrigger {
            common: common.clone(),
            delay,
        }),
        Trigger::Registration(RegistrationTrigger {
            common: common.clone(),
            delay,
        }),
        Trigger::Idle(IdleTrigger {
            common: common.clone(),
        }),
        Trigger::Time(TimeTrigger {
            common: common.clone(),
            random_delay: delay,
        }),
        Trigger::Event(EventTrigger {
            common: common.clone(),
            subscription:
                "<QueryList><Query Id=\"0\"><Select Path=\"System\">*</Select></Query></QueryList>"
                    .into(),
            value_queries: [("value".into(), "Event/EventData/Data".into())].into(),
            delay,
            period_of_occurrence: delay,
            number_of_occurrences: Some(2),
            matching_element: Some("Event/System/EventID".into()),
        }),
        Trigger::Logon(LogonTrigger {
            common: common.clone(),
            user_id: Some("fixture".into()),
            delay,
        }),
        Trigger::SessionStateChange(SessionStateChangeTrigger {
            common: common.clone(),
            state_change: SessionStateChange::SessionLock,
            user_id: Some("fixture".into()),
            delay,
        }),
        Trigger::Daily(DailyTrigger {
            common: common.clone(),
            days_interval: 2,
            random_delay: delay,
        }),
        Trigger::Weekly(WeeklyTrigger {
            common: common.clone(),
            weeks_interval: 2,
            days_of_week: [Weekday::Monday, Weekday::Friday].into(),
            random_delay: delay,
        }),
        Trigger::Monthly(MonthlyTrigger {
            common: common.clone(),
            days_of_month: [1, 31].into(),
            months: [Month::January, Month::December].into(),
            run_on_last_day: true,
            random_delay: delay,
        }),
        Trigger::MonthlyDow(MonthlyDowTrigger {
            common,
            weeks_of_month: [WeekOfMonth::First].into(),
            days_of_week: [Weekday::Friday].into(),
            months: [Month::February].into(),
            run_on_last_week: true,
            random_delay: delay,
        }),
    ];
    for trigger in triggers {
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("fixture.exe")));
        definition.triggers.push(trigger).expect("one trigger");
        round_trip(&definition);
    }
}

#[test]
fn settings_and_schema_versions_round_trip() {
    for schema in [
        TaskSchemaVersion::V1_2,
        TaskSchemaVersion::V1_3,
        TaskSchemaVersion::V1_4,
        TaskSchemaVersion::V1_5,
        TaskSchemaVersion::V1_6,
    ] {
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("fixture.exe")));
        definition.schema_version = schema;
        definition.settings = TaskSettings {
            allow_demand_start: false,
            restart_on_failure: Some(RestartPolicy {
                interval: TaskDuration::from_secs(60),
                count: 2,
            }),
            multiple_instances: MultipleInstancesPolicy::Queue,
            disallow_start_if_on_batteries: false,
            stop_if_going_on_batteries: false,
            allow_hard_terminate: false,
            start_when_available: true,
            network: Some(NetworkSettings {
                id: Some(uuid::Uuid::nil().to_string()),
                name: Some("fixture".into()),
            }),
            execution_time_limit: TaskLimit::Unlimited,
            enabled: false,
            delete_expired_after: Some(TaskDuration::from_secs(60)),
            priority: 3,
            idle: Some(IdleSettings {
                duration: TaskDuration::from_secs(60),
                wait_timeout: TaskDuration::from_secs(120),
                stop_on_idle_end: false,
                restart_on_idle: true,
            }),
            wake_to_run: true,
            hidden: true,
            use_unified_scheduling_engine: true,
            disallow_start_on_remote_app_session: true,
            maintenance: Some(MaintenanceSettings {
                period: TaskDuration::from_secs(86400),
                deadline: TaskDuration::from_secs(172_800),
                exclusive: true,
            }),
            volatile: true,
        };
        if definition.schema_version != TaskSchemaVersion::V1_6 {
            assert!(
                !definition.validate().is_valid(),
                "older schemas cannot represent every new setting"
            );
            assert!(
                !to_string(&definition)
                    .expect_err("invalid schema must not serialize")
                    .diagnostics()
                    .is_empty()
            );
            definition.settings.maintenance = None;
            definition.settings.volatile = false;
            definition.settings.use_unified_scheduling_engine = false;
            definition.settings.disallow_start_on_remote_app_session = false;
        }
        round_trip(&definition);
    }
}

#[test]
fn logon_identities_preserve_semantics() {
    for (identity, logon_type) in [
        (PrincipalIdentity::None, LogonType::None),
        (
            PrincipalIdentity::User("fixture".into()),
            LogonType::Password,
        ),
        (PrincipalIdentity::User("fixture".into()), LogonType::S4u),
        (
            PrincipalIdentity::User("fixture".into()),
            LogonType::InteractiveToken,
        ),
        (
            PrincipalIdentity::User("fixture".into()),
            LogonType::InteractiveTokenOrPassword,
        ),
        (PrincipalIdentity::Group("fixture".into()), LogonType::Group),
        (
            PrincipalIdentity::ServiceAccount(ServiceAccount::LocalSystem),
            LogonType::ServiceAccount,
        ),
        (
            PrincipalIdentity::ServiceAccount(ServiceAccount::LocalService),
            LogonType::ServiceAccount,
        ),
        (
            PrincipalIdentity::ServiceAccount(ServiceAccount::NetworkService),
            LogonType::ServiceAccount,
        ),
    ] {
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("fixture.exe")));
        definition.principal.identity = identity;
        definition.principal.logon_type = logon_type;
        definition.principal.display_name = Some("fixture".into());
        definition.principal.run_level = RunLevel::HighestAvailable;
        definition.principal.process_token_sid_type = ProcessTokenSidType::Unrestricted;
        round_trip(&definition);
    }
}
