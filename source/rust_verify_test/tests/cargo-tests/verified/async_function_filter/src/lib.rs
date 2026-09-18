use vstd::prelude::*;

mod focused {
    use vstd::prelude::*;

    #[verus_verify]
    pub struct S;

    impl S {
        #[verus_spec]
        pub async fn target(&mut self) {}
    }
}
