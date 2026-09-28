pub(crate) mod csi {
    #[allow(clippy::enum_variant_names)]
    pub mod v1 {
        use tonic::include_proto;
        include_proto!("csi.v1");
    }
}
