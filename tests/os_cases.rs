// chuong trinh cham vao he dieu hanh: doc ghi file, dong lenh, tien trinh con.
//
// Moi ca la tests/cases/os/NAME.pump nam canh tests/cases/os/NAME.out, giong
// het cac ca `run`, chi khac mot cho: chuong trinh nhan hai doi so, mot thu
// muc nhap de no ghi vao va duong dan cua chinh `pump` de no chay thu mot
// tien trinh con. Khong ca nao duoc ghi vao trong cay nguon.

mod support;

macro_rules! os_cases {
    ($($name:ident,)*) => {
        $(
            #[test]
            fn $name() {
                support::check_os_case(stringify!($name));
            }
        )*

        #[test]
        fn every_case_is_registered() {
            support::check_registry("os", "out", &[$(stringify!($name)),*]);
        }
    };
}

os_cases! {
    // doc ghi mot file text, va mot file thieu la loi bat duoc.
    files_text,
    // byte di thanh [int], ke ca cai byte 0 va cai byte 255.
    files_bytes,
    // os.args, tu `pump run FILE -- ...`.
    command_line,
    // chay `pump` nhu tien trinh con va xem ma thoat.
    subprocess,
}
