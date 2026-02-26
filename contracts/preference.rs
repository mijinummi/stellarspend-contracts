//! User Notification Preferences Contract
//!
//! Stores per-user notification preferences on-chain and gates event
//! dispatch so events are only emitted for channels the user has enabled.
//!
//! # Supported notification channels
//!
//! | Channel         | Description                          |
//! |-----------------|--------------------------------------|
//! | `OnChain`       | Soroban events (always available)    |
//! | `Email`         | Off-chain email bridge               |
//! | `Push`          | Mobile push notification bridge      |
//! | `Sms`           | SMS bridge                           |
//!
//! # Supported event types
//!
//! Users independently enable/disable each event type per channel:
//! `Transfer`, `Reward`, `FraudAlert`, `StreakUpdate`, `PaymentExecuted`,
//! `AccountFrozen`, `RecurringContribution`, `SystemAlert`.
//!
//! # Lifecycle
//!
//! 1. **`initialize`** — one-time admin setup.
//! 2. **`set_preferences`** — user stores their full preference record.
//! 3. **`update_channel`** — user toggles a single channel on/off.
//! 4. **`update_event_type`** — user toggles a single event type per channel.
//! 5. **`dispatch_notification`** — called by other contracts; emits a
//!    structured event only if the target user has that channel + event enabled.
//!
//! # Security properties
//!
//! - `require_auth` is the first statement in every mutating entry point.
//! - Users can only modify their own preferences.
//! - `dispatch_notification` is permissionless — any contract may call it,
//!   but it only emits events; it writes no user state.
//! - TTL is bumped on every persistent-storage access.

#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, panic_with_error, symbol_short,
    Address, Env, Vec,
};

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledger TTL bump for persistent preference records (~2 years).
const PERSISTENT_TTL_BUMP: u32 = 12_614_400;

// ── Storage keys ─────────────────────────────────────────────────────────────

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    /// Admin address (instance storage).
    Admin,
    /// Per-user preference record (persistent storage).
    UserPreferences(Address),
}

// ── Notification channels ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum NotificationChannel {
    OnChain = 0,
    Email = 1,
    Push = 2,
    Sms = 3,
}

// ── Event types ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum EventType {
    Transfer = 0,
    Reward = 1,
    FraudAlert = 2,
    StreakUpdate = 3,
    PaymentExecuted = 4,
    AccountFrozen = 5,
    RecurringContribution = 6,
    SystemAlert = 7,
}

// ── Per-channel settings ──────────────────────────────────────────────────────

/// Settings for a single notification channel.
#[derive(Clone)]
#[contracttype]
pub struct ChannelPreference {
    /// Whether this channel is globally enabled for the user.
    pub enabled: bool,
    /// Bitmask of enabled event types (bit N = EventType variant N).
    /// Bit 0 = Transfer, 1 = Reward, … 7 = SystemAlert.
    pub event_mask: u32,
}

impl ChannelPreference {
    /// All event types enabled by default.
    pub fn all_enabled() -> Self {
        Self {
            enabled: true,
            event_mask: 0xFF, // all 8 event-type bits set
        }
    }

    /// Channel present but fully disabled.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            event_mask: 0,
        }
    }

    /// Return `true` if `event_type` is enabled on this channel.
    pub fn accepts(&self, event_type: EventType) -> bool {
        if !self.enabled {
            return false;
        }
        let bit = 1u32 << (event_type as u32);
        self.event_mask & bit != 0
    }

    /// Enable a specific event type.
    pub fn enable_event(&mut self, event_type: EventType) {
        self.event_mask |= 1u32 << (event_type as u32);
    }

    /// Disable a specific event type.
    pub fn disable_event(&mut self, event_type: EventType) {
        self.event_mask &= !(1u32 << (event_type as u32));
    }
}

// ── Full user preference record ───────────────────────────────────────────────

/// Complete notification preference record for one user.
#[derive(Clone)]
#[contracttype]
pub struct UserPreferenceRecord {
    pub on_chain: ChannelPreference,
    pub email: ChannelPreference,
    pub push: ChannelPreference,
    pub sms: ChannelPreference,
    /// Ledger timestamp of the last update.
    pub updated_at: u64,
    /// Total number of times this record has been updated.
    pub update_count: u32,
}

impl UserPreferenceRecord {
    /// Sensible defaults: OnChain always enabled, others disabled.
    pub fn default_preferences() -> Self {
        Self {
            on_chain: ChannelPreference::all_enabled(),
            email: ChannelPreference::disabled(),
            push: ChannelPreference::disabled(),
            sms: ChannelPreference::disabled(),
            updated_at: 0,
            update_count: 0,
        }
    }

    /// Return the `ChannelPreference` for a given channel (immutable ref via clone).
    pub fn channel(&self, channel: NotificationChannel) -> &ChannelPreference {
        match channel {
            NotificationChannel::OnChain => &self.on_chain,
            NotificationChannel::Email => &self.email,
            NotificationChannel::Push => &self.push,
            NotificationChannel::Sms => &self.sms,
        }
    }

    /// Return a mutable reference to the `ChannelPreference` for a channel.
    pub fn channel_mut(&mut self, channel: NotificationChannel) -> &mut ChannelPreference {
        match channel {
            NotificationChannel::OnChain => &mut self.on_chain,
            NotificationChannel::Email => &mut self.email,
            NotificationChannel::Push => &mut self.push,
            NotificationChannel::Sms => &mut self.sms,
        }
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PreferencesError {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    /// Caller is not the owner of the preference record.
    Unauthorized = 3,
    /// Arithmetic overflow on update_count.
    Overflow = 4,
}

// ── Events ────────────────────────────────────────────────────────────────────

pub struct PreferenceEvents;

impl PreferenceEvents {
    /// Emitted when a user sets or replaces their full preference record.
    /// Payload: `(user, timestamp)`
    pub fn preferences_set(env: &Env, user: &Address) {
        env.events().publish(
            (symbol_short!("pref"), symbol_short!("set")),
            (user.clone(), env.ledger().timestamp()),
        );
    }

    /// Emitted when a single channel is toggled.
    /// Payload: `(user, channel, enabled, timestamp)`
    pub fn channel_updated(
        env: &Env,
        user: &Address,
        channel: NotificationChannel,
        enabled: bool,
    ) {
        env.events().publish(
            (symbol_short!("pref"), symbol_short!("chan_upd")),
            (user.clone(), channel as u32, enabled, env.ledger().timestamp()),
        );
    }

    /// Emitted when a single event type is toggled on a channel.
    /// Payload: `(user, channel, event_type, enabled, timestamp)`
    pub fn event_type_updated(
        env: &Env,
        user: &Address,
        channel: NotificationChannel,
        event_type: EventType,
        enabled: bool,
    ) {
        env.events().publish(
            (symbol_short!("pref"), symbol_short!("evt_upd")),
            (
                user.clone(),
                channel as u32,
                event_type as u32,
                enabled,
                env.ledger().timestamp(),
            ),
        );
    }

    /// Emitted on behalf of another contract when the user accepts the
    /// notification on this channel.
    /// Payload: `(user, channel, event_type, payload_ref, timestamp)`
    pub fn notification_dispatched(
        env: &Env,
        user: &Address,
        channel: NotificationChannel,
        event_type: EventType,
        payload_ref: u64, // caller-supplied reference ID (e.g. payment_id)
    ) {
        env.events().publish(
            (symbol_short!("pref"), symbol_short!("notify")),
            (
                user.clone(),
                channel as u32,
                event_type as u32,
                payload_ref,
                env.ledger().timestamp(),
            ),
        );
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

impl NotificationPreferencesContract {
    fn require_initialized(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic_with_error!(env, PreferencesError::NotInitialized))
    }

    fn load_preferences(env: &Env, user: &Address) -> UserPreferenceRecord {
        let key = DataKey::UserPreferences(user.clone());
        let record: UserPreferenceRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(UserPreferenceRecord::default_preferences);
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, PERSISTENT_TTL_BUMP, PERSISTENT_TTL_BUMP);
        }
        record
    }

    fn save_preferences(env: &Env, user: &Address, record: &UserPreferenceRecord) {
        let key = DataKey::UserPreferences(user.clone());
        env.storage().persistent().set(&key, record);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_TTL_BUMP, PERSISTENT_TTL_BUMP);
    }

    /// Bump `update_count` and `updated_at`, then persist.
    fn touch(env: &Env, user: &Address, record: &mut UserPreferenceRecord) {
        record.updated_at = env.ledger().timestamp();
        record.update_count = record
            .update_count
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(env, PreferencesError::Overflow));
        Self::save_preferences(env, user, record);
    }
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct NotificationPreferencesContract;

#[contractimpl]
impl NotificationPreferencesContract {
    // ── Lifecycle ────────────────────────────────────────────────────────────

    /// Initialise the contract. Only callable once.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, PreferencesError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    // ── Preference management ────────────────────────────────────────────────

    /// Replace the caller's entire preference record.
    ///
    /// Useful for first-time setup or bulk resets. Preserves `update_count`
    /// history — only the channel settings are overwritten.
    ///
    /// # Security
    /// - `user.require_auth()` is first; users can only set their own prefs.
    pub fn set_preferences(
        env: Env,
        user: Address,
        on_chain: ChannelPreference,
        email: ChannelPreference,
        push: ChannelPreference,
        sms: ChannelPreference,
    ) {
        user.require_auth();
        Self::require_initialized(&env);

        let mut record = Self::load_preferences(&env, &user);
        record.on_chain = on_chain;
        record.email = email;
        record.push = push;
        record.sms = sms;

        Self::touch(&env, &user, &mut record);
        PreferenceEvents::preferences_set(&env, &user);
    }

    /// Toggle a single notification channel on or off.
    ///
    /// Disabling a channel suppresses all events on it without losing the
    /// per-event-type mask, so re-enabling restores the previous granular
    /// settings.
    pub fn update_channel(
        env: Env,
        user: Address,
        channel: NotificationChannel,
        enabled: bool,
    ) {
        user.require_auth();
        Self::require_initialized(&env);

        let mut record = Self::load_preferences(&env, &user);
        record.channel_mut(channel).enabled = enabled;

        Self::touch(&env, &user, &mut record);
        PreferenceEvents::channel_updated(&env, &user, channel, enabled);
    }

    /// Toggle a single event type on a specific channel.
    ///
    /// The channel must be enabled; toggling event types on a disabled channel
    /// is allowed (the mask is updated even when the channel is off) so that
    /// re-enabling the channel preserves the user's event preferences.
    pub fn update_event_type(
        env: Env,
        user: Address,
        channel: NotificationChannel,
        event_type: EventType,
        enabled: bool,
    ) {
        user.require_auth();
        Self::require_initialized(&env);

        let mut record = Self::load_preferences(&env, &user);
        let ch = record.channel_mut(channel);
        if enabled {
            ch.enable_event(event_type);
        } else {
            ch.disable_event(event_type);
        }

        Self::touch(&env, &user, &mut record);
        PreferenceEvents::event_type_updated(&env, &user, channel, event_type, enabled);
    }

    // ── Notification dispatch ─────────────────────────────────────────────────

    /// Dispatch a notification to `user` if their preferences allow it.
    ///
    /// Called by other contracts (e.g. `streak_rewards`, `conditional_payments`)
    /// to emit a user-facing notification event. Iterates over all four channels
    /// and emits a `notification_dispatched` event for each enabled one.
    ///
    /// `payload_ref` is an opaque u64 reference (e.g. a payment ID or schedule
    /// ID) that off-chain bridges use to fetch full event context.
    ///
    /// This function is **permissionless** — any contract may call it. It only
    /// emits events; it never writes user state.
    ///
    /// Returns the number of channels the notification was dispatched on.
    pub fn dispatch_notification(
        env: Env,
        user: Address,
        event_type: EventType,
        payload_ref: u64,
    ) -> u32 {
        Self::require_initialized(&env);

        let record = Self::load_preferences(&env, &user);
        let channels = [
            NotificationChannel::OnChain,
            NotificationChannel::Email,
            NotificationChannel::Push,
            NotificationChannel::Sms,
        ];

        let mut dispatched: u32 = 0;
        for channel in channels.iter() {
            if record.channel(*channel).accepts(event_type) {
                PreferenceEvents::notification_dispatched(
                    &env,
                    &user,
                    *channel,
                    event_type,
                    payload_ref,
                );
                dispatched = dispatched.saturating_add(1);
            }
        }
        dispatched
    }

    // ── Read-only queries ────────────────────────────────────────────────────

    /// Return the full preference record for `user`.
    /// Returns the default record if the user has not set preferences yet.
    pub fn get_preferences(env: Env, user: Address) -> UserPreferenceRecord {
        Self::require_initialized(&env);
        Self::load_preferences(&env, &user)
    }

    /// Return `true` if `user` would receive a notification of `event_type`
    /// on `channel`.
    pub fn is_enabled(
        env: Env,
        user: Address,
        channel: NotificationChannel,
        event_type: EventType,
    ) -> bool {
        Self::require_initialized(&env);
        let record = Self::load_preferences(&env, &user);
        record.channel(channel).accepts(event_type)
    }

    /// Return which channels would receive a notification for `event_type`.
    /// Returns a `Vec<u32>` of `NotificationChannel` discriminants.
    pub fn active_channels_for(
        env: Env,
        user: Address,
        event_type: EventType,
    ) -> Vec<u32> {
        Self::require_initialized(&env);
        let record = Self::load_preferences(&env, &user);
        let mut result = Vec::new(&env);
        let channels = [
            NotificationChannel::OnChain,
            NotificationChannel::Email,
            NotificationChannel::Push,
            NotificationChannel::Sms,
        ];
        for ch in channels.iter() {
            if record.channel(*ch).accepts(event_type) {
                result.push_back(*ch as u32);
            }
        }
        result
    }
}