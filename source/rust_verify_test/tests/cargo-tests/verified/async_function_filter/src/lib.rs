#![no_std]

mod focused {
    use vstd::prelude::*;

    #[verus_verify]
    pub struct S;

    impl S {
        #[verus_spec]
        async fn step(&mut self) {}

        #[verus_spec]
        pub async fn target(&mut self) {
            self.step().await;
        }
    }
}
