use crate::esi::EsiClient;
use serde::Deserialize;

/// A dogma attribute on a type.
#[derive(Debug, Clone, Deserialize)]
pub struct DogmaAttribute {
    pub attribute_id: i32,
    pub value: f64,
}

/// A dogma effect on a type.
#[derive(Debug, Clone, Deserialize)]
pub struct DogmaEffect {
    pub effect_id: i32,
    pub is_default: bool,
}

/// Detailed information about an item type.
#[derive(Debug, Clone, Deserialize)]
pub struct TypeInfo {
    pub type_id: i32,
    pub name: String,
    pub description: String,
    pub group_id: i32,
    pub market_group_id: Option<i32>,
    pub capacity: Option<f64>,
    pub mass: Option<f64>,
    pub radius: Option<f64>,
    pub volume: Option<f64>,
    pub packaged_volume: Option<f64>,
    pub portion_size: Option<i32>,
    pub base_price: Option<f64>,
    pub published: bool,
    pub icon_id: Option<i32>,
    pub graphic_id: Option<i32>,
    pub dogma_attributes: Option<Vec<DogmaAttribute>>,
    pub dogma_effects: Option<Vec<DogmaEffect>>,
}

/// Endpoints under `/universe/`.
pub struct UniverseEndpoint<'a> {
    pub(crate) client: &'a EsiClient,
}

impl<'a> UniverseEndpoint<'a> {
    /// Get all published type IDs.
    ///
    /// `GET /universe/types/`
    ///
    /// Results are automatically paginated and aggregated.
    pub async fn types(&self) -> Result<Vec<i32>, crate::esi::error::EsiError> {
        self.client.get("universe/types/", &[]).await
    }

    /// Get detailed information about a specific type.
    ///
    /// `GET /universe/types/{type_id}/`
    pub async fn type_info(&self, type_id: i32) -> Result<TypeInfo, crate::esi::error::EsiError> {
        let path = format!("universe/types/{}/", type_id);
        self.client.get(&path, &[]).await
    }
}
