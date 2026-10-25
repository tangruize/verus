#![feature(rustc_private)]
#[macro_use]
mod common;
use common::*;

test_verify_one_file! {
    #[test] test_basic_async_function_ensures_pass verus_code! {
        use vstd::prelude::*;
        async fn foo() -> (ret: usize)
            ensures
                ret == 1,
        {
            1
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] test_basic_async_function_ensures_fail verus_code! {
        use vstd::prelude::*;
        async fn foo() -> (ret: usize)
            ensures
                ret == 2,  // FAILS
        {
            1
        }
    } => Err(e) => assert_one_fails(e)
}

test_verify_one_file! {
    #[test] test_basic_async_function_and_await verus_code! {
        use vstd::prelude::*;
        async fn foo() -> (ret: usize)
            ensures
                ret == 1,
        {
            1
        }
        async fn bar() {
            let future = foo();
            let ret = future.await;
            assert(ret == 1);
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] test_basic_async_function_util verus_code! {
        use vstd::prelude::*;
        use vstd::future::*;
        async fn foo() -> (ret: usize)
            ensures
                ret == 1,
        {
            1
        }
        async fn bar() {
            let future = foo();
            assert(future.awaited() ==> future@ == 1);
            let ret = future.await;
            assert(ret == 1);
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] test_basic_async_function_lifetime_fail verus_code! {
        use vstd::prelude::*;
        async fn foo(x :&usize) -> (ret: usize)
            ensures
                ret == 1,
        {
            1
        }
        async fn bar() {
            let mut x = 233;
            let future = foo(&x);
            x = 2333;
            let ret = future.await;
            x = 2333;
        }
    } => Err(err) => assert_rust_error_msg(err, "cannot assign to `x` because it is borrowed")
}

test_verify_one_file! {
    #[test] test_basic_async_function_nested_pass verus_code! {
        use vstd::prelude::*;
        use core::future::*;
        use vstd::future::*;
        async fn foo() -> (ret: usize)
            ensures
                ret == 233,
        {
            233
        }

        async fn foo_of_foo() -> (ret: impl Future<Output = usize>)
            ensures
                ret.awaited() ==> ret@ == 233,
        {
            foo()
        }
        async fn bar() {
            let future_of_future = foo_of_foo();
            let ret = future_of_future.await.await;
            assert(ret == 233);
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] test_await_outside_of_async_function_fail verus_code! {
        use vstd::prelude::*;
        async fn foo() -> (ret: usize)
            ensures
                ret == 233,
        {
            233
        }

        fn bar() {
            let future = foo();
            future.await;
        }
    } => Err(err) => assert_rust_error_msg(err, "`await` is only allowed inside `async` functions and blocks")
}

test_verify_one_file! {
    #[test] test_async_function_external_specification verus_code! {
        use vstd::prelude::*;
        #[verifier(external)]
        async fn negate_bool(b: bool, x: u8) -> bool {
            !b
        }

        #[verifier(external_fn_specification)]
        async fn negate_bool_requires_ensures(b: bool, x: u8) -> (ret_b: bool)
            requires x != 0,
            ensures ret_b == !b
        {
            negate_bool(b, x).await
        }

        async fn foo(){
            let future = negate_bool(true, 1);
            let ret = future.await;
            assert(ret == false);
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] test_async_function_mut_ref_ok verus_code! {
        use vstd::prelude::*;
        pub async fn bar(x: &mut usize) -> (ret: ())
            ensures
                *final(x) == 2333,
        {
            *x = 2333;
        }

        async fn foo(){
            let mut x = 233;
            let future = bar(&mut x);
            future.await;
            assert(x == 2333);
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] test_async_trait_default_method verus_code! {
        use vstd::prelude::*;
        use core::time::Duration;

        struct Error;

        trait StateUnit {
            async fn advance_time(&mut self, _duration: Duration) -> Result<(), Error> {
                Ok(())
            }
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] test_async_trait_default_method_body_is_checked verus_code! {
        use vstd::prelude::*;

        trait StateUnit {
            async fn advance_time(&mut self) {
                assert(false); // FAILS
            }
        }
    } => Err(err) => assert_one_fails(err)
}

test_verify_one_file! {
    #[test] async_block_in_closure verus_code! {
        use vstd::prelude::*;

        struct Owned(usize);

        async fn receive(owned: Owned) -> (result: Owned)
            ensures result.0 == owned.0,
        {
            owned
        }

        fn retain<T>(value: T) -> T {
            value
        }

        async fn test() {
            let make_future = |owned: Owned| async move {
                let expected = owned.0;
                let received = receive(owned).await;
                assert(received.0 == expected);
                received.0
            };
            let _future = retain(make_future(Owned(7)));
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] async_block_body_is_checked verus_code! {
        use vstd::prelude::*;

        async fn test() {
            let make_future = || async move {
                assert(false); // FAILS
            };
            let _future = make_future();
        }
    } => Err(err) => assert_vir_error_msg(err, "assertion failed")
}

test_verify_one_file! {
    #[test] async_block_future_bound_remains_unsupported verus_code! {
        use core::future::Future;
        use vstd::prelude::*;

        fn retain_future<F: Future<Output = usize>>(future: F) -> F {
            future
        }

        async fn test() {
            let future = retain_future(async { 7usize });
            let _value: usize = future.await;
        }
    } => Err(err) => assert_vir_error_msg(err, "Verus does not recognize this trait bound")
}

test_verify_one_file! {
    #[test] async_block_direct_await verus_code! {
        use vstd::prelude::*;

        async fn test() {
            let _value: usize = (async { 7usize }).await;
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] async_block_direct_await_does_not_prove_false verus_code! {
        use vstd::prelude::*;

        async fn test() {
            let _value: usize = (async { 7usize }).await;
            assert(false); // FAILS
        }
    } => Err(err) => assert_vir_error_msg(err, "assertion failed")
}

test_verify_one_file! {
    #[test] async_dyn_future_output_relation verus_code! {
        use core::future::Future;
        use vstd::prelude::*;

        fn projected_identity<F: Future + ?Sized>(
            _future: &F,
            output: F::Output,
        ) -> (result: F::Output)
            ensures result == output,
        {
            output
        }

        fn retain_projected_output<'a>(
            future: &'a (dyn Future<Output = usize> + 'a),
            output: usize,
        ) -> (result: usize)
            ensures result == output,
        {
            projected_identity(future, output)
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] async_dyn_future_output_relation_is_checked verus_code! {
        use core::future::Future;
        use vstd::prelude::*;

        fn projected_identity<F: Future + ?Sized>(
            _future: &F,
            output: F::Output,
        ) -> (result: F::Output)
            ensures result == output,
        {
            output
        }

        fn reject_wrong_projected_output<'a>(
            future: &'a (dyn Future<Output = usize> + 'a),
            output: usize,
        ) -> (result: usize)
            ensures result == output + 1, // FAILS
        {
            projected_identity(future, output)
        }
    } => Err(err) => assert_vir_error_msg(err, "postcondition not satisfied")
}

test_verify_one_file! {
    #[test] async_dyn_future_auto_trait_is_not_dropped verus_code! {
        use core::future::Future;
        use vstd::prelude::*;

        fn reject_auto_trait<'a>(
            _future: &'a (dyn Future<Output = usize> + Send + 'a),
        ) {
        }
    } => Err(err) => assert_vir_error_msg(err, "dyn with more that one trait")
}

test_verify_one_file! {
    #[test] async_dyn_future_parameter verus_code! {
        use core::future::Future;
        use vstd::prelude::*;

        fn retain_type(_future: &dyn Future<Output = usize>) {
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] async_dyn_future_coercions verus_code! {
        use core::future::Future;
        use vstd::prelude::*;

        fn as_dyn<'a, F: Future<Output = usize> + 'a>(
            future: &'a F,
        ) -> &'a (dyn Future<Output = usize> + 'a) {
            future
        }

        fn as_dyn_mut<'a, F: Future<Output = usize> + 'a>(
            future: &'a mut F,
        ) -> &'a mut (dyn Future<Output = usize> + 'a) {
            future
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] unit_return_value_issue2321 verus_code! {
        use vstd::prelude::*;
        async fn set_zero(x: &mut usize)
            ensures *final(x) == 0,
        {
            *x = 0;
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] async_impl_future_trait_issue2521 code! {
        use std::future::Future;
        use vstd::prelude::*;

        trait T {
            fn f() -> impl Future;
        }

        struct S;

        impl T for S {
          fn f() -> impl Future { async {} }
        }

        verus! {
          async fn foo();
        }
    } => Ok(())
}

test_verify_one_file! {
    #[test] async_spec_fn verus_code! {
        async spec fn foo() -> bool {
            true
        }
    } => Err(err) => assert_vir_error_msg(err, "async is only supported for 'exec' functions")
}

test_verify_one_file! {
    #[test] async_proof_fn verus_code! {
        async proof fn foo() -> bool {
            true
        }
    } => Err(err) => assert_vir_error_msg(err, "async is only supported for 'exec' functions")
}

test_verify_one_file! {
    #[test] async_with_returns verus_code! {
        use vstd::prelude::*;

        async fn foo() -> (ret: u64)
            returns 3u64
        {
            return 3;
        }

        async fn foo_fails() -> (ret: u64)
            returns 3u64
        {
            return 4; // FAILS
        }

        async fn get_foo() {
            let f = foo().await;
            assert(f == 3);
        }

        async fn get_foo_fails() {
            let f = foo().await;
            assert(f == 4); // FAILS
        }
    } => Err(err) => assert_fails(err, 2)
}

test_verify_one_file! {
    #[test] async_with_allow_in_spec verus_code! {
        spec fn foo_spec() -> u64 {
            3
        }

        #[verifier::allow_in_spec]
        async fn foo() -> (ret: u64)
            returns 3u64
        {
            return 3;
        }
    } => Err(err) => assert_vir_error_msg(err, "async is not supported together with `allow_in_spec`")
}

test_verify_one_file! {
    #[test] async_with_mismatch_type code! {
        #[verus::internal(verus_macro)]
        async fn foo() -> u64 {
            ::vstd::prelude::ensures(|ret: u32|
                    [::vstd::prelude::spec_eq(ret, 3u64)]);
            return 3;
        }
    } => Err(err) => assert_vir_error_msg(err, "return type does not match binder in ensures")
}
