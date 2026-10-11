use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::AccountRef;
use crate::cache_model::{self, Cache, Causes};
use crate::forecast::{self, Forecast};
pub use crate::limits::BudgetView;
use crate::limits::{self, LimitWindow};
use crate::pricing::PriceTable;
use crate::store::{Store, StoredEvent};
use crate::timeline::{STEP_KIND, Timeline};

pub const FIVE_HOUR: &str = "five_hour";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Live {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub cache: Cache,
    pub forecast: Option<Forecast>,
    pub limits: Vec<LimitWindow>,
    #[serde(default)]
    pub account: Option<AccountRef>,
    #[serde(default)]
    pub budget: Option<BudgetView>,
}

impl Live {
    pub fn read(
        store: &Store,
        prices: &PriceTable,
        session: &str,
        measure: Option<&StoredEvent>,
        now_ms: i64,
    ) -> Result<Self> {
        let requests = store.session_requests(session)?;
        let steps = store.events_since(session, STEP_KIND, Timeline::steps_from_ms(&requests))?;
        let timeline = Timeline::build(&requests, &steps);
        let measured: Option<Value> =
            measure.and_then(|event| serde_json::from_str(&event.json).ok());
        let latest = timeline.latest_main();
        let model = latest.map(|request| request.model.clone()).or_else(|| {
            measured
                .as_ref()
                .and_then(|data| data.get("model")?.as_str().map(String::from))
        });
        let reported: Vec<&str> = measured
            .as_ref()
            .and_then(|data| data.get("rateLimits")?.as_array())
            .into_iter()
            .flatten()
            .filter_map(|limit| limit.get("kind")?.as_str())
            .collect();
        let account_id = store.session_account(session)?;
        let account = match account_id.as_deref() {
            Some(id) => store.account(id)?.map(AccountRef::from),
            None => None,
        };
        let mut limits = limits::windows(
            &limits::recent_samples(store, now_ms)?,
            &limits::stored_fit(store)?,
            model.as_deref(),
            now_ms,
            &reported,
            account_id.as_deref(),
        );
        let budget = match account_id.as_deref() {
            Some(id) => store
                .budget(id)?
                .filter(|budget| budget.amount_usd.is_finite() && budget.amount_usd > 0.0)
                .map(|budget| {
                    let period_start_ms = limits::month_start_utc(now_ms);
                    Ok::<_, anyhow::Error>(BudgetView {
                        amount_usd: budget.amount_usd,
                        local_usd: store
                            .account_spend_since(id, period_start_ms.div_euclid(1000))?,
                        period_start_ms,
                        resets_at_ms: limits::next_month_start_utc(now_ms),
                    })
                })
                .transpose()?,
            None => None,
        };
        if let Some(budget) = &budget
            && !limits
                .iter()
                .any(|window| window.window == limits::SPEND_LIMIT)
        {
            limits.push(limits::budget_window(budget, model.as_deref(), now_ms));
        }
        let causes = Causes::read(store, session, i64::MIN, i64::MAX)?;
        let cache = cache_model::model(
            &timeline,
            &causes.context(prices, !reported.is_empty(), now_ms),
        );
        let pct_per_usd_5h = limits
            .iter()
            .find(|window| window.window == FIVE_HOUR)
            .and_then(|window| window.calibration.pct_per_usd);
        Ok(Self {
            model,
            effort: latest.and_then(|request| request.effort.clone()),
            forecast: forecast::next_turn(&timeline.main(), prices, cache.ttl, pct_per_usd_5h),
            cache,
            limits,
            account,
            budget,
        })
    }
}
