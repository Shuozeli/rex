//! Generated types for the rex contracts plus the embedded descriptor set.
//!
//! Module nesting mirrors the proto packages so prost's cross-package `super::` paths resolve.

pub mod rex {
    pub mod schema {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/rex.schema.v1.rs"));
        }
        pub mod article {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.article.v1.rs"));
            }
        }
        pub mod feed {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.feed.v1.rs"));
            }
        }
        pub mod search {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.search.v1.rs"));
            }
        }
        pub mod commerce {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.commerce.v1.rs"));
            }
        }
        pub mod social {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.social.v1.rs"));
            }
        }
        pub mod media {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.media.v1.rs"));
            }
        }
        pub mod common {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.common.v1.rs"));
            }
        }
        pub mod howto {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.howto.v1.rs"));
            }
        }
        pub mod faq {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.faq.v1.rs"));
            }
        }
        pub mod event {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.event.v1.rs"));
            }
        }
        pub mod place {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.place.v1.rs"));
            }
        }
        pub mod job {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.job.v1.rs"));
            }
        }
        pub mod app {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.app.v1.rs"));
            }
        }
        pub mod finance {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.finance.v1.rs"));
            }
        }
        pub mod creative {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/rex.schema.creative.v1.rs"));
            }
        }
    }
    pub mod extract {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/rex.extract.v1.rs"));
        }
    }
    pub mod bundle {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/rex.bundle.v1.rs"));
        }
    }
}

pub use rex::extract::v1 as extract;
pub use rex::schema::v1 as annotations;

/// FileDescriptorSet of every rex proto and its imports (well-known types, descriptor.proto).
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rex.fds"));

/// Full name of the recipe message.
pub const EXTRACTION_RECIPE: &str = "rex.extract.v1.ExtractionRecipe";
/// Full name of the output envelope message.
pub const EXTRACTION: &str = "rex.extract.v1.Extraction";

/// Decodes the embedded descriptor set into a reflection pool.
pub fn descriptor_pool() -> Result<prost_reflect::DescriptorPool, prost_reflect::DescriptorError> {
    prost_reflect::DescriptorPool::decode(FILE_DESCRIPTOR_SET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_pool_contains_contract_messages() {
        // Arrange / Act
        let pool = descriptor_pool().expect("pool decodes");

        // Assert
        for name in [
            EXTRACTION_RECIPE,
            EXTRACTION,
            "rex.schema.article.v1.Article",
            "rex.schema.feed.v1.Feed",
            "rex.bundle.v1.SignedIndex",
        ] {
            assert!(pool.get_message_by_name(name).is_some(), "missing {name}");
        }
        assert!(pool.get_extension_by_name("rex.schema.v1.field").is_some());
    }
}
