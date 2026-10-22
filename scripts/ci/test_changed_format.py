"""Run changed-line format checks against real clang-format 18 in isolated Git repos."""

from pathlib import Path
import subprocess

from test_linelen import GitFixture

CHECKER = Path(__file__).with_name("changed_format.py")
STYLE = (
    "BasedOnStyle: LLVM\nIndentWidth: 4\nBreakBeforeBraces: Linux\nColumnLimit: 100\n"
    "AllowShortFunctionsOnASingleLine: Empty\n"
)


class ChangedFormatTests(GitFixture):
    def setUp(self):
        super().setUp()
        self.write(".clang-format", STYLE)
        self.commit()
        self.base = self.git("rev-parse", "HEAD").strip()

    def format_check(self, code=0, base=None):
        result = subprocess.run(
            ["python3", str(CHECKER), "--root", str(self.root), "--base", base or self.base],
            text=True,
            capture_output=True,
            timeout=20,
        )
        self.assertEqual(result.returncode, code, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def test_new_bad_broker_code_fails(self):
        self.write("broker/new.c", "int f( void){return 1;}\n")
        self.commit()
        self.format_check(code=1)

    def test_new_bad_cuda_code_fails(self):
        self.write("cuda/new.cu", "__global__ void f( int *p){*p=1;}\n")
        self.commit()
        self.format_check(code=1)

    def test_forced_color_cannot_hide_bad_broker_code(self):
        self.git("config", "color.diff", "always")
        self.write("broker/new.c", "int f( void){return 1;}\n")
        self.commit()
        self.format_check(code=1)

    def test_forced_color_cannot_hide_bad_cuda_code(self):
        self.git("config", "color.diff", "always")
        self.write("cuda/new.cu", "__global__ void f( int *p){*p=1;}\n")
        self.commit()
        self.format_check(code=1)

    def test_new_bad_kf3_code_fails(self):
        self.write("qemu/hw/misc/kf3/new.c", "int f( void){return 1;}\n")
        self.commit()
        self.format_check(code=1)

    def test_formatted_added_code_passes(self):
        self.write("broker/new.c", "int f(void)\n{\n    return 1;\n}\n")
        self.commit()
        self.format_check()

    def test_unchanged_legacy_code_is_not_formatted(self):
        self.write("broker/old.c", "int f( void){return 1;}\n")
        self.commit()
        self.base = self.git("rev-parse", "HEAD").strip()
        self.write("tools/a.py", "print(1)\n")
        self.commit()
        self.format_check()

    def test_changed_broker_file_retains_unedited_legacy_functions(self):
        legacy = "int old( void){return 7;}\n\nint spacer(void)\n{\n    return 0;\n}\n"
        self.write("broker/old.c", legacy)
        self.commit()
        self.base = self.git("rev-parse", "HEAD").strip()
        self.write("broker/old.c", legacy + "\nint added(void)\n{\n    return 2;\n}\n")
        self.commit()
        self.format_check()

    def test_changed_cuda_file_retains_unedited_legacy_functions(self):
        legacy = "__global__ void old( int *p){*p=7;}\n\nint spacer(void)\n{\n    return 0;\n}\n"
        self.write("cuda/old.cu", legacy)
        self.commit()
        self.base = self.git("rev-parse", "HEAD").strip()
        self.write("cuda/old.cu", legacy + "\nint added(void)\n{\n    return 2;\n}\n")
        self.commit()
        self.format_check()

    def test_archive_and_generated_code_are_not_formatted(self):
        self.write("broker/archive/new.c", "int f( void){return 1;}\n")
        self.write("cuda/generated.cu", "// @generated; do not edit\nint f( void){return 1;}\n")
        self.commit()
        self.format_check()

    def test_check_never_modifies_source_files(self):
        self.write("broker/new.c", "int f( void){return 1;}\n")
        self.commit()
        self.format_check(code=1)
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_missing_base_is_an_infrastructure_error(self):
        self.format_check(code=2, base="missing-ref")
