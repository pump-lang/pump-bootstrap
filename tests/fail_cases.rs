// chuong trinh BAT BUOC phai bi tu choi, moi cai ghim mot ma loi va ghim ca
// cach dien dat lam cho thong bao do co ich.
//
// Moi ca la tests/cases/fail/NAME.pump nam canh tests/cases/fail/NAME.err.
// Dinh dang cua .err viet o support::Expectation. Thu muc con trong
// tests/cases/fail chua may module phu ma ca nhieu file can import; cho check
// danh sach chi nhin file thoi.

mod support;

macro_rules! fail_cases {
    ($($name:ident,)*) => {
        $(
            #[test]
            fn $name() {
                support::check_fail_case(stringify!($name));
            }
        )*

        #[test]
        fn every_case_is_registered() {
            support::check_registry("fail", "err", &[$(stringify!($name)),*]);
        }
    };
}

fail_cases! {
    // ---- bay cai ma de bai ke ten ----
    assign_to_const,
    non_exhaustive_match,
    optional_without_narrowing,
    propagate_from_non_failable,
    struct_literal_in_if_header,
    visibility_violation,
    type_mismatch,

    // ---- muc tu vung ----
    newline_in_string,
    integer_literal_too_large,
    non_ascii_identifier,
    backslash_outside_import,
    char_literal_too_long,

    // ---- muc cu phap ----
    chained_comparison,
    assignment_in_expression,
    statement_has_no_effect,
    top_level_let,
    nested_function,
    one_tuple,
    import_after_declaration,
    map_literal_in_if_header,
    set_literal_spelling,
    positional_after_named,
    required_after_default,
    variadic_not_last,
    float_pattern,
    interpolation_in_pattern,

    // ---- giai ten ----
    unknown_identifier,
    unknown_field,
    duplicate_declaration,
    duplicate_parameter,
    shadow_predeclared_type,
    unused_import,
    private_field_access,
    this_outside_method,
    missing_main,
    break_outside_loop,
    module_not_found,
    circular_import,
    duplicate_import_binding,

    // ---- kieu ----
    wrong_argument_count,
    condition_not_bool,
    no_implicit_conversion,
    string_not_indexable,
    tuple_index_out_of_range,
    wrong_type_argument_count,
    missing_struct_field,
    unknown_struct_field,
    interface_not_satisfied,
    missing_return,
    unreachable_match_arm,
    or_pattern_binding_mismatch,
    assign_to_loop_binding,
    assign_to_this,
    method_on_unbounded_generic,
    implements_generic_subject,
    interface_method_has_body,
    enum_without_variants,

    // ---- cua vao he dieu hanh ----
    file_bytes_are_not_a_string,

    // ---- optional va loi ----
    nested_optional,
    narrowing_defeated_by_assignment,
    error_type_in_binding,
    unhandled_error,
    fail_outside_failable,
    catch_on_non_failable,

    // ---- bieu thuc hang ----
    division_by_zero_constant,
    constant_overflow,
    not_a_constant_expression,
}
