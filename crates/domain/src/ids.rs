//! Domain-specific UUID identifiers.
use serde::{Deserialize, Serialize};
use uuid::Uuid;
macro_rules! id {
    ($name:ident,$doc:literal) => {
        #[doc=$doc]
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl $name {
            /** Constructs from an already validated UUID. */
            pub const fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }
            /** Returns the UUID. */
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }
    };
}
id!(PlantId, "Stable plant identity.");
id!(ProfileId, "Plant profile template identity.");
id!(WateringEventId, "Watering ledger event identity.");
