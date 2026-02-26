//! Tests for contracts/preferences.rs
//!
//! Run with: `cargo test --test preferences_tests`
//!
//! Test taxonomy
//! ─────────────
//! happy_*     — correct flows
//! neg_*       — invalid inputs / calls that must be rejected
//! edge_*      — boundary / bitmask / combination corner cases
//! auth_*      — authorization guard tests
//! dispatch_*  — notification dispatch gating tests

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env,
};

use crate::preferences::{
    ChannelPreference, EventType, NotificationChannel, NotificationPreferencesContract,
    NotificationPreferencesContractClient, PreferencesError, UserPreferenceRecord,
};

// ── Harness ───────────────────────────────────────────────────────────────────

struct Ctx {
    env: Env,
    client: NotificationPreferencesContractClient<'static>,
    admin: Address,
}

impl Ctx {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, NotificationPreferencesContract);
        let client: NotificationPreferencesContractClient =
            NotificationPreferencesContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.initialize(&admin);

        let client: NotificationPreferencesContractClient<'static> =
            unsafe { core::mem::transmute(client) };

        Self { env, client, admin }
    }

    fn set_timestamp(&self, ts: u64) {
        self.env.ledger().set(LedgerInfo {
            timestamp: ts,
            ..self.env.ledger().get()
        });
    }

    /// Set full preferences: OnChain enabled (all events), others disabled.
    fn set_default_prefs(&self, user: &Address) {
        self.client.set_preferences(
            user,
            &ChannelPreference { enabled: true, event_mask: 0xFF },
            &ChannelPreference { enabled: false, event_mask: 0 },
            &ChannelPreference { enabled: false, event_mask: 0 },
            &ChannelPreference { enabled: false, event_mask: 0 },
        );
    }
}

// ── Initialization ────────────────────────────────────────────────────────────

#[test]
fn happy_initialize_succeeds() {
    let ctx = Ctx::new();
    // Contract initialized — get_preferences on a fresh user returns defaults.
    let user = Address::generate(&ctx.env);
    let prefs = ctx.client.get_preferences(&user);
    // Default: on_chain enabled with all events, others disabled.
    assert!(prefs.on_chain.enabled);
    assert!(!prefs.email.enabled);
    assert!(!prefs.push.enabled);
    assert!(!prefs.sms.enabled);
}

#[test]
#[should_panic(expected = "AlreadyInitialized")]
fn neg_reinit_blocked() {
    let ctx = Ctx::new();
    ctx.client.initialize(&ctx.admin);
}

// ── set_preferences ───────────────────────────────────────────────────────────

#[test]
fn happy_set_preferences_stores_all_channels() {
    let ctx = Ctx::new();
    ctx.set_timestamp(1_000);
    let user = Address::generate(&ctx.env);

    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0x03 },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: true, event_mask: 0x10 },
    );

    let prefs = ctx.client.get_preferences(&user);
    assert!(prefs.on_chain.enabled);
    assert_eq!(prefs.on_chain.event_mask, 0xFF);
    assert!(prefs.email.enabled);
    assert_eq!(prefs.email.event_mask, 0x03);
    assert!(!prefs.push.enabled);
    assert!(prefs.sms.enabled);
    assert_eq!(prefs.sms.event_mask, 0x10);
    assert_eq!(prefs.updated_at, 1_000);
    assert_eq!(prefs.update_count, 1);
}

#[test]
fn happy_set_preferences_increments_update_count() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    for _ in 0..5 {
        ctx.set_default_prefs(&user);
    }

    let prefs = ctx.client.get_preferences(&user);
    assert_eq!(prefs.update_count, 5);
}

#[test]
fn happy_set_preferences_overwrites_previous() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    // First set: email enabled.
    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );
    assert!(ctx.client.get_preferences(&user).email.enabled);

    // Second set: email disabled.
    ctx.set_default_prefs(&user);
    assert!(!ctx.client.get_preferences(&user).email.enabled);
}

// ── update_channel ────────────────────────────────────────────────────────────

#[test]
fn happy_update_channel_toggles_enabled() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);
    ctx.set_default_prefs(&user);

    // Enable email.
    ctx.client.update_channel(&user, &NotificationChannel::Email, &true);
    assert!(ctx.client.get_preferences(&user).email.enabled);

    // Disable it again.
    ctx.client.update_channel(&user, &NotificationChannel::Email, &false);
    assert!(!ctx.client.get_preferences(&user).email.enabled);
}

#[test]
fn happy_update_channel_preserves_event_mask() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    // Set email with a specific mask.
    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0x05 },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    // Disable the channel.
    ctx.client.update_channel(&user, &NotificationChannel::Email, &false);
    let prefs = ctx.client.get_preferences(&user);
    // Channel disabled but mask preserved.
    assert!(!prefs.email.enabled);
    assert_eq!(prefs.email.event_mask, 0x05);

    // Re-enable — mask should still be 0x05.
    ctx.client.update_channel(&user, &NotificationChannel::Email, &true);
    let prefs2 = ctx.client.get_preferences(&user);
    assert!(prefs2.email.enabled);
    assert_eq!(prefs2.email.event_mask, 0x05);
}

#[test]
fn happy_update_channel_all_four_channels() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    for channel in [
        NotificationChannel::OnChain,
        NotificationChannel::Email,
        NotificationChannel::Push,
        NotificationChannel::Sms,
    ] {
        ctx.client.update_channel(&user, &channel, &true);
    }

    let prefs = ctx.client.get_preferences(&user);
    assert!(prefs.on_chain.enabled);
    assert!(prefs.email.enabled);
    assert!(prefs.push.enabled);
    assert!(prefs.sms.enabled);
}

// ── update_event_type ─────────────────────────────────────────────────────────

#[test]
fn happy_update_event_type_enables_single_bit() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    // Start with email channel enabled but no event types.
    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    ctx.client.update_event_type(
        &user,
        &NotificationChannel::Email,
        &EventType::Reward,
        &true,
    );

    assert!(ctx.client.is_enabled(
        &user,
        &NotificationChannel::Email,
        &EventType::Reward,
    ));
    // Other event types still off.
    assert!(!ctx.client.is_enabled(
        &user,
        &NotificationChannel::Email,
        &EventType::Transfer,
    ));
}

#[test]
fn happy_update_event_type_disables_single_bit() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    // Disable FraudAlert on email.
    ctx.client.update_event_type(
        &user,
        &NotificationChannel::Email,
        &EventType::FraudAlert,
        &false,
    );

    assert!(!ctx.client.is_enabled(
        &user,
        &NotificationChannel::Email,
        &EventType::FraudAlert,
    ));
    // Others still on.
    assert!(ctx.client.is_enabled(
        &user,
        &NotificationChannel::Email,
        &EventType::Transfer,
    ));
}

#[test]
fn happy_event_mask_updated_even_when_channel_disabled() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);
    ctx.set_default_prefs(&user); // email disabled

    // Enable Reward event type on the disabled email channel.
    ctx.client.update_event_type(
        &user,
        &NotificationChannel::Email,
        &EventType::Reward,
        &true,
    );

    let prefs = ctx.client.get_preferences(&user);
    // Channel still disabled, but the bit is set.
    assert!(!prefs.email.enabled);
    let reward_bit = 1u32 << (EventType::Reward as u32);
    assert!(prefs.email.event_mask & reward_bit != 0);
}

// ── is_enabled / active_channels_for ─────────────────────────────────────────

#[test]
fn happy_is_enabled_respects_channel_and_event() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0x01 }, // only Transfer
        &ChannelPreference { enabled: false, event_mask: 0xFF }, // disabled
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    // OnChain: all events enabled.
    assert!(ctx.client.is_enabled(&user, &NotificationChannel::OnChain, &EventType::Transfer));
    assert!(ctx.client.is_enabled(&user, &NotificationChannel::OnChain, &EventType::SystemAlert));

    // Email: only Transfer.
    assert!(ctx.client.is_enabled(&user, &NotificationChannel::Email, &EventType::Transfer));
    assert!(!ctx.client.is_enabled(&user, &NotificationChannel::Email, &EventType::Reward));

    // Push: channel disabled → nothing passes.
    assert!(!ctx.client.is_enabled(&user, &NotificationChannel::Push, &EventType::Transfer));

    // SMS: disabled and no mask.
    assert!(!ctx.client.is_enabled(&user, &NotificationChannel::Sms, &EventType::Transfer));
}

#[test]
fn happy_active_channels_for_returns_correct_set() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0x02 }, // Reward only
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    // Transfer: OnChain + Push (email only has Reward).
    let channels = ctx.client.active_channels_for(&user, &EventType::Transfer);
    assert_eq!(channels.len(), 2);
    assert!(channels.contains(&(NotificationChannel::OnChain as u32)));
    assert!(channels.contains(&(NotificationChannel::Push as u32)));

    // Reward: OnChain + Email + Push.
    let channels2 = ctx.client.active_channels_for(&user, &EventType::Reward);
    assert_eq!(channels2.len(), 3);
}

// ── dispatch_notification ─────────────────────────────────────────────────────

#[test]
fn dispatch_returns_count_of_channels_notified() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    // Enable OnChain and Push for Transfer.
    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    let count = ctx.client.dispatch_notification(&user, &EventType::Transfer, &42u64);
    assert_eq!(count, 2); // OnChain + Push
}

#[test]
fn dispatch_zero_when_no_channels_match() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    // All channels disabled.
    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    let count = ctx.client.dispatch_notification(&user, &EventType::Reward, &1u64);
    assert_eq!(count, 0);
}

#[test]
fn dispatch_default_user_gets_only_on_chain() {
    let ctx = Ctx::new();
    // User who has never set preferences — defaults apply.
    let user = Address::generate(&ctx.env);

    let count = ctx.client.dispatch_notification(&user, &EventType::Transfer, &0u64);
    assert_eq!(count, 1); // only OnChain by default
}

#[test]
fn dispatch_respects_event_type_mask_not_just_channel() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    // Email enabled but only for Reward (bit 1 = 0x02).
    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0x02 }, // Reward only
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    // Transfer: only OnChain (email mask excludes Transfer).
    let t_count = ctx.client.dispatch_notification(&user, &EventType::Transfer, &1u64);
    assert_eq!(t_count, 1);

    // Reward: OnChain + Email.
    let r_count = ctx.client.dispatch_notification(&user, &EventType::Reward, &2u64);
    assert_eq!(r_count, 2);
}

#[test]
fn dispatch_all_channels_all_events() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
    );

    for event in [
        EventType::Transfer,
        EventType::Reward,
        EventType::FraudAlert,
        EventType::StreakUpdate,
        EventType::PaymentExecuted,
        EventType::AccountFrozen,
        EventType::RecurringContribution,
        EventType::SystemAlert,
    ] {
        let count = ctx.client.dispatch_notification(&user, &event, &0u64);
        assert_eq!(count, 4, "expected 4 channels for event {:?}", event);
    }
}

// ── edge_*: bitmask boundary cases ───────────────────────────────────────────

#[test]
fn edge_each_event_type_maps_to_unique_bit() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    let events = [
        EventType::Transfer,
        EventType::Reward,
        EventType::FraudAlert,
        EventType::StreakUpdate,
        EventType::PaymentExecuted,
        EventType::AccountFrozen,
        EventType::RecurringContribution,
        EventType::SystemAlert,
    ];

    for (i, &event) in events.iter().enumerate() {
        // Enable exactly one event type via a 1-bit mask.
        let mask = 1u32 << i;
        ctx.client.set_preferences(
            &user,
            &ChannelPreference { enabled: true, event_mask: 0xFF },
            &ChannelPreference { enabled: true, event_mask: mask },
            &ChannelPreference { enabled: false, event_mask: 0 },
            &ChannelPreference { enabled: false, event_mask: 0 },
        );

        // Only this event should pass on email.
        assert!(
            ctx.client.is_enabled(&user, &NotificationChannel::Email, &event),
            "bit {} should enable event {:?}",
            i,
            event
        );

        // Every other event type should fail on email.
        for (j, &other_event) in events.iter().enumerate() {
            if j != i {
                assert!(
                    !ctx.client.is_enabled(&user, &NotificationChannel::Email, &other_event),
                    "bit {} should NOT enable event {:?}",
                    i,
                    other_event
                );
            }
        }
    }
}

#[test]
fn edge_zero_mask_disables_all_events_even_when_channel_enabled() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0x00 }, // enabled but no events
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    for event in [EventType::Transfer, EventType::Reward, EventType::SystemAlert] {
        assert!(!ctx.client.is_enabled(&user, &NotificationChannel::Email, &event));
    }
}

#[test]
fn edge_full_mask_enables_all_events() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    ctx.client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );

    for event in [
        EventType::Transfer,
        EventType::Reward,
        EventType::FraudAlert,
        EventType::StreakUpdate,
        EventType::PaymentExecuted,
        EventType::AccountFrozen,
        EventType::RecurringContribution,
        EventType::SystemAlert,
    ] {
        assert!(ctx.client.is_enabled(&user, &NotificationChannel::Email, &event));
    }
}

#[test]
fn edge_toggle_event_type_idempotent() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);
    ctx.set_default_prefs(&user);

    // Enable Transfer on email twice — should not create duplicate bits.
    ctx.client.update_event_type(&user, &NotificationChannel::Email, &EventType::Transfer, &true);
    ctx.client.update_event_type(&user, &NotificationChannel::Email, &EventType::Transfer, &true);

    let prefs = ctx.client.get_preferences(&user);
    let expected_bit = 1u32 << (EventType::Transfer as u32);
    // Bit should be set exactly once (idempotent OR).
    assert_eq!(prefs.email.event_mask & expected_bit, expected_bit);

    // Disable twice — bit should be clear.
    ctx.client.update_event_type(&user, &NotificationChannel::Email, &EventType::Transfer, &false);
    ctx.client.update_event_type(&user, &NotificationChannel::Email, &EventType::Transfer, &false);

    let prefs2 = ctx.client.get_preferences(&user);
    assert_eq!(prefs2.email.event_mask & expected_bit, 0);
}

#[test]
fn edge_update_count_tracks_all_mutation_types() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    ctx.set_default_prefs(&user);                                                           // 1
    ctx.client.update_channel(&user, &NotificationChannel::Email, &true);                   // 2
    ctx.client.update_event_type(&user, &NotificationChannel::Push, &EventType::Reward, &true); // 3
    ctx.set_default_prefs(&user);                                                           // 4

    let prefs = ctx.client.get_preferences(&user);
    assert_eq!(prefs.update_count, 4);
}

#[test]
fn edge_updated_at_reflects_latest_mutation_timestamp() {
    let ctx = Ctx::new();
    let user = Address::generate(&ctx.env);

    ctx.set_timestamp(1_000);
    ctx.set_default_prefs(&user);

    ctx.set_timestamp(5_000);
    ctx.client.update_channel(&user, &NotificationChannel::Push, &true);

    let prefs = ctx.client.get_preferences(&user);
    assert_eq!(prefs.updated_at, 5_000);
}

// ── edge_*: multi-user isolation ──────────────────────────────────────────────

#[test]
fn edge_users_have_independent_preferences() {
    let ctx = Ctx::new();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);

    // Alice enables all channels; Bob keeps defaults.
    ctx.client.set_preferences(
        &alice,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: true, event_mask: 0xFF },
    );

    // Bob: only OnChain (default).
    let alice_count = ctx.client.dispatch_notification(&alice, &EventType::Transfer, &1u64);
    let bob_count = ctx.client.dispatch_notification(&bob, &EventType::Transfer, &2u64);

    assert_eq!(alice_count, 4);
    assert_eq!(bob_count, 1);
}

#[test]
fn edge_modifying_alice_prefs_does_not_affect_bob() {
    let ctx = Ctx::new();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);

    ctx.set_default_prefs(&alice);
    ctx.set_default_prefs(&bob);

    // Alice enables push.
    ctx.client.update_channel(&alice, &NotificationChannel::Push, &true);

    assert!(ctx.client.get_preferences(&alice).push.enabled);
    assert!(!ctx.client.get_preferences(&bob).push.enabled);
}

// ── auth_*: authorization guards ─────────────────────────────────────────────

#[test]
#[should_panic]
fn auth_set_preferences_requires_auth() {
    let env = Env::default();
    // No mock_all_auths.
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, NotificationPreferencesContract);
    let client = NotificationPreferencesContractClient::new(&env, &contract_id);
    {
        env.mock_all_auths();
        client.initialize(&admin);
    }
    let user = Address::generate(&env);
    client.set_preferences(
        &user,
        &ChannelPreference { enabled: true, event_mask: 0xFF },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
        &ChannelPreference { enabled: false, event_mask: 0 },
    );
}

#[test]
#[should_panic]
fn auth_update_channel_requires_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, NotificationPreferencesContract);
    let client = NotificationPreferencesContractClient::new(&env, &contract_id);
    {
        env.mock_all_auths();
        client.initialize(&admin);
    }
    let user = Address::generate(&env);
    // No auth — must panic.
    client.update_channel(&user, &NotificationChannel::Email, &true);
}

#[test]
#[should_panic]
fn auth_update_event_type_requires_auth() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, NotificationPreferencesContract);
    let client = NotificationPreferencesContractClient::new(&env, &contract_id);
    {
        env.mock_all_auths();
        client.initialize(&admin);
    }
    let user = Address::generate(&env);
    client.update_event_type(&user, &NotificationChannel::Push, &EventType::Reward, &true);
}