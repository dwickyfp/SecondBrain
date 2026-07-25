#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_smoke_test() {
        assert_eq!(env!("CARGO_PKG_NAME"), "secondbrain-core");
    }
}
