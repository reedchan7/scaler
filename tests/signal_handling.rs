use scaler::core::run_loop::InterruptPlan;

#[test]
fn interrupt_plan_is_sigint_then_sigterm_then_sigkill() {
    let plan = InterruptPlan::default();

    assert_eq!(plan.sigterm_after().as_secs(), 2);
    assert_eq!(plan.sigkill_after().as_secs(), 5);
}
