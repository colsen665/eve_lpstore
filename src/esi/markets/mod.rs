use crate::esi::EsiClient;
use serde::Deserialize;

/// A market order.
#[derive(Debug, Clone, Deserialize)]
pub struct MarketOrder {
    pub order_id: i64,
    pub type_id: i32,
    pub location_id: i64,
    pub volume_remain: i32,
    pub volume_total: i32,
    pub min_volume: i32,
    pub price: f64,
    pub is_buy_order: bool,
    pub duration: i32,
    pub issued: String,
    pub range: String,
}

/// Endpoints under `/markets/`.
pub struct MarketsEndpoint<'a> {
    pub(crate) client: &'a EsiClient,
}

impl<'a> MarketsEndpoint<'a> {
    /// Get market orders in a region, optionally filtered by item type.
    ///
    /// `GET /markets/{region_id}/orders/?type_id={type_id}`
    ///
    /// Results are automatically paginated and aggregated.
    pub async fn orders(
        &self,
        region_id: i32,
        type_id: Option<i32>,
    ) -> Result<Vec<MarketOrder>, crate::esi::error::EsiError> {
        let path = format!("markets/{}/orders/", region_id);
        let query: Vec<(&str, String)> = type_id
            .map(|t| vec![("type_id", t.to_string())])
            .unwrap_or_default();
        self.client.get(&path, &query).await
    }

    /// Get market orders in a region, filtered by order type.
    ///
    /// `GET /markets/{region_id}/orders/?order_type={order_type}`
    pub async fn orders_by_type(
        &self,
        region_id: i32,
        order_type: &str,
    ) -> Result<Vec<MarketOrder>, crate::esi::error::EsiError> {
        let path = format!("markets/{}/orders/", region_id);
        let query: [(&str, String); 1] = [("order_type", order_type.to_string())];
        self.client.get(&path, &query).await
    }
}
