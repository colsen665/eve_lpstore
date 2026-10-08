use crate::esi::EsiClient;
use serde::Deserialize;

/// An item required by a loyalty store offer.
#[derive(Debug, Clone, Deserialize)]
pub struct RequiredItem {
    pub type_id: i32,
    pub quantity: i32,
}

/// A loyalty store offer.
#[derive(Debug, Clone, Deserialize)]
pub struct LoyaltyOffer {
    pub offer_id: i32,
    pub type_id: i32,
    pub quantity: i32,
    pub lp_cost: i32,
    pub isk_cost: f64,
    pub required_items: Vec<RequiredItem>,
}

/// Endpoints under `/loyalty/`.
pub struct LoyaltyEndpoint<'a> {
    pub(crate) client: &'a EsiClient,
}

impl<'a> LoyaltyEndpoint<'a> {
    /// Get loyalty store offers for a corporation.
    ///
    /// `GET /loyalty/stores/{corporation_id}/offers/`
    pub async fn offers(
        &self,
        corporation_id: i32,
    ) -> Result<Vec<LoyaltyOffer>, crate::esi::error::EsiError> {
        let path = format!("loyalty/stores/{}/offers/", corporation_id);
        self.client.get(&path, &[]).await
    }
}
