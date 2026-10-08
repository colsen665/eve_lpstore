use crate::esi::EsiClient;

/// Endpoints under `/corporations/`.
pub struct CorporationsEndpoint<'a> {
    pub(crate) client: &'a EsiClient,
}

impl<'a> CorporationsEndpoint<'a> {
    /// Get a list of NPC corporation IDs.
    ///
    /// `GET /corporations/npccorps/`
    pub async fn npc_corps(&self) -> Result<Vec<i32>, crate::esi::error::EsiError> {
        self.client.get("corporations/npccorps/", &[]).await
    }
}
