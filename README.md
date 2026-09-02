Pump là một ngôn ngữ biên dịch nhỏ, cú pháp đơn giản, có GC.

Dịch bằng Cranelift ra mã máy thật: `pump run file.pump` chạy thẳng trong bộ nhớ, `pump build file.pump` ra một cái .exe.

Cú pháp đầy đủ nằm trong pump-syntax.txt, grammar ở grammar/pump.ebnf, ví dụ ở examples/.

## Đây là bản đầu tiên

Repo này là Pump viết bằng Rust, dựng trên Cranelift. Nó là bản đầu tiên, và bây giờ nó chỉ còn
đúng một việc: dịch stage0 cho trình biên dịch thật, cái được viết bằng chính Pump.

Trình biên dịch đó nằm ở đây, và nếu bạn tới tìm ngôn ngữ Pump thì nên sang bên đó:

  https://github.com/pump-lang/pump

Bên này giữ lại vì một chuỗi bootstrap cần một điểm bắt đầu không tự dịch được chính nó. Nó
không còn được phát triển thêm nữa; sửa gì ở đây cũng chỉ để stage0 vẫn dựng được. Repo chính
ghim nó theo SHA trong `pump.toml`, nên lịch sử bên này không được viết lại.

## This is the first one

This repository is the original Pump compiler, written in Rust on top of Cranelift. It has one
job left: building stage0 for the real compiler, which is written in Pump itself.

That compiler lives here, and if you came looking for the language, go there:

  https://github.com/pump-lang/pump

This one stays because a bootstrap chain needs a starting point that cannot compile itself. It
is not developed any further; anything that changes here changes so that stage0 still builds.
The main repository pins it by SHA in `pump.toml`, so the history on this side is never
rewritten.
