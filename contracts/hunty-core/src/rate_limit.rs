use crate::errors::HuntErrorCode;
use crate::storage::Storage;
use crate::types::RateLimitStatus;
use soroban_sdk::{contracttype, Address, Env, Symbol};

pub const SECONDS_PER_DAY: u64 = 86_400;
pub const DEFAULT_HUNT_CREATION_LIMIT: u32 = 10;

/// TTL extension target for rate limit entries (~30 days).
const RATE_LIMIT_TTL: u32 = 30 * 24 * 60 * 60;
/// Threshold below which we extend the TTP (~15 days).
const RATE_LIMIT_TTL_THRESHOLD: u32 = 15 * 24 * 60 * 60;

/// Namespace used to avoid collisions with other features keying by a bare `Address`.
const RATE_LIMIT_NAMESPACE: &str = "HRATE";

/// Legacy namespace used before the fix, for migration of existing entries.
const RATE_LIMIT_LEGACY_NAMESPACE: &str = "HRATE_LEGACY";

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttpe]
pub struct RateLimitData {
    pub day: u64,
    pub count: u32,
}

pub struct RateLimiter;

impl RateLimiter {
    fn key(env: &Env, creator: &Address) -> (Symbol, Address) {
        (Symbol::new(env, RATE_LIMIT_NAMESPACE), creator.clone())
    }

    fn legacy_key(env: &Env, creator: &Address) -> (Symbol, Address) {
        (Symbol::new(env, RATE_LIMIT_LEGACY_NAMESPACE), creator.clone())
    }

    /// Read the rate limit data for a creator, migrating legacy entries if needed.
    fn read(env: &Env, creator: &Address) -> Option<RateLimitData> {
        if let::Ok(data) = env
            .storage()
            .persistent()
            .get::<(Symbol, Address), RateLimitData>(&key(env, creator))
        {
            return Some(data);
        }

        // Migrate existing entries that were stored under the bare address.
        if let::Ok(data) = env
            .storage()
            .persistent()
            .get::<Address, RateLimitData>(creator)
        {
            env
                .storage()
                .persistent()
                .set(&legacy_key(env, creator), &data);
            env
                .storage()
                .persistent()
                .remove(&creator);
            return Some(data);
        }

        None
    }

    fn write(env: &Env, creator: &Address, data: &RateLimitData) {
        let key = key(env, creator);
        env.storage().persistent().set(&key, data);
        env.storage()
            .persistent()
            .extend_ttl(&key, RATE_LIMIT_TTL_THRESHOLD, RATE_LIMIT_TTL);
    }

    pub fn check_and_increment(
        env: &Env,
        creator: &Address,
        now: u64,
    ) -> Result<(), HuntErrorCode> {
        let day = now / SECONDS_PER_DAY;
        let limit = Storage::get_effective_hunt_creation_limit(env, creator);
        let mut data = self::read(env, creator).unwrap_or(RateLimitData { day, count: 0 });

        if data.day != day {
            data.day = day;
            data.count = 0;
        }

        if data.count >= limit {
            return Err(HuntErrorCode::RateLimitExceeded);
        }

        data.count += 1;
        self::write(env, creator, &data);
        Ok(()
    }

    #[allow(dead_code)]
    pub fn get_status(env: &Env, creator: &Address, now: u64) -> RateLimitStatus {
        let day = now / SECONDS_PER_DAY;
        let limit = Storage::get_effective_hunt_creation_limit(env, creator);
        let data = self::read(env, creator).unwrap_or(RateLimitData { day, count: 0 });

        let count = if data.day == day { data.count } else { 0 };
        let cooldown_seconds = if count >= limit {
            (day + 1)
                .saturating_mul(SECONDS_PER_DAY)
                .saturating_sub(now)
        } else {
            0
        };
        RateLimitStatus {
            creations_today: count,
            daily_limit: limit,
            cooldown_seconds: cooldown_seconds,
        }
    }

    #[allow(dead_code)]
    pub fn require_rate_limit_admin(env: &Env, admin: &Address) -> Result<(), HuntErrorCode> {
        admin.require_auth();
        let stored = Storage::get_rate_limit_admin(env).ok_or(HuntErrorCode::Unauthorized)?;
        if stored != *admin {
            return Err(HuntErrorCode::Unauthorized);
        }
        Ok(())
    }
}
