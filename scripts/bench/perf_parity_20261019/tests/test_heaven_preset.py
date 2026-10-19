from pathlib import Path
import re
import shlex


def launch_options():
    source = (Path(__file__).resolve().parents[1] / 'heaven_ui.ps1').read_text()
    arguments = shlex.split(re.search(r"\$args2='([^']*)'", source)[1])
    return dict(zip(arguments[::2], arguments[1::2]))


def test_requested_vsync_is_explicit_in_the_heaven_command():
    assert launch_options().get('-video_vsync') == '1'


def test_an_inherited_vsync_off_config_cannot_change_the_requested_preset():
    settings = {'video_vsync': '0', 'video_app': 'opengl', 'video_width': '800'}
    settings.update({key.lstrip('-'): value for key, value in launch_options().items()})
    assert settings['video_vsync'] == '1'
    assert settings['video_app'] == 'direct3d11'
    assert settings['video_width'] == '1920'
    assert settings['video_height'] == '1080'
    assert 'QUALITY_ULTRA' in settings['extern_define']
