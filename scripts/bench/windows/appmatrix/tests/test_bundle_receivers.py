# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""RV2 receiver falsifiers: fully self-consistent hostile artifacts must refuse."""
import io
import tarfile
import zipfile
from pathlib import Path
import tempfile
import unittest

import helpers as H
import package_bundle as B
import bundle_iso as I
import bundle_repro as R
from receiver_fixtures import Artifact, ImagePair, digest, directory_entries


class ReviewedPublication(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.a=Artifact(Path(self.tmp.name))

    def denied(self):
        rc,store,factory=self.a.publish()
        self.assertEqual(rc,1);self.assertEqual(store.objects,{})
        self.assertEqual(store.calls,[]);factory.assert_not_called()

    def test_healthy_reviewed_artifact_publishes(self):
        rc,store,factory=self.a.publish()
        self.assertEqual(rc,0);self.assertEqual(len(store.objects),2);factory.assert_called_once()

    def test_cli_healthy_reviewed_artifact_passes(self):
        self.assertEqual(self.a.verify_cli(), 0)

    def test_cli_changed_package_pins_with_stolen_hash_refused(self):
        self.a.rewrite(self.changed_package)
        self.assertEqual(self.a.verify_cli(), 1)

    def test_cli_self_valid_different_package_recipe_refused(self):
        self.a.rewrite(self.changed_package, claimed='derive')
        self.assertEqual(self.a.verify_cli(), 1)

    def test_cli_self_valid_different_toolchain_recipe_refused(self):
        self.a.rewrite(self.changed_toolchain, claimed='derive')
        self.assertEqual(self.a.verify_cli(), 1)

    def test_cli_payload_with_original_pin_refused(self):
        self.a.rewrite(lambda c,e:c.update({'pkg/sevenzip/7zr.exe':b'x'*22}))
        self.assertEqual(self.a.verify_cli(), 1)

    def test_cli_guest_bytes_with_refreshed_receipt_refused(self):
        self.a.rewrite(lambda c,e:c.update({'guest/kf_stage.ps1':b'unreviewed guest script'}))
        self.assertEqual(self.a.verify_cli(), 1)

    @staticmethod
    def changed_package(contents, embedded):
        data=b'new pinned vendor artifact'
        embedded['packages'][0]['files'][0].update(url='https://other.invalid/replaced.exe',**digest(data))
        contents['pkg/sevenzip/7zr.exe']=data

    @staticmethod
    def changed_toolchain(contents, embedded):
        embedded['toolchain']['hosts']['x86_64-linux']['sha256']='f'*64

    def test_changed_embedded_pins_with_stolen_reviewed_hash_refused(self):
        self.a.rewrite(self.changed_package);self.denied()

    def test_changed_toolchain_with_stolen_reviewed_hash_refused(self):
        self.a.rewrite(self.changed_toolchain);self.denied()

    def test_self_valid_different_package_recipe_not_authorized(self):
        self.a.rewrite(self.changed_package,claimed='derive');self.denied()

    def test_self_valid_different_toolchain_recipe_not_authorized(self):
        self.a.rewrite(self.changed_toolchain,claimed='derive');self.denied()

    def test_verifier_derives_package_recipe_identity(self):
        self.a.rewrite(self.changed_package)
        with self.assertRaisesRegex(ValueError,'recipe.*identity|input.*identity'):B.verify_image(self.a.iso,self.a.receipt)

    def test_verifier_derives_toolchain_recipe_identity(self):
        self.a.rewrite(self.changed_toolchain)
        with self.assertRaisesRegex(ValueError,'recipe.*identity|input.*identity'):B.verify_image(self.a.iso,self.a.receipt)

    def test_package_payload_with_original_pin_refused_before_store(self):
        self.a.rewrite(lambda c,e:c.update({'pkg/sevenzip/7zr.exe':b'x'*22}))
        with self.assertRaisesRegex(B.InputDrift, 'INPUT_DRIFT package sevenzip'):
            B.verify_image(self.a.iso, self.a.receipt)
        self.denied()

    def test_resized_payload_with_original_pin_refused_before_store(self):
        self.a.rewrite(lambda c,e:c.update({'pkg/sevenzip/7zr.exe':b'replacement with a different length'}))
        with self.assertRaisesRegex(B.InputDrift, 'INPUT_DRIFT package sevenzip'):
            B.verify_image(self.a.iso, self.a.receipt)
        self.denied()

    def test_recipe_source_audit_cannot_claim_another_builder(self):
        self.a.receipt['recipe_sha256']['package_bundle.py'] = 'e'*64
        self.a.rewrite(lambda c,e:None); self.denied()

    def test_recipe_source_audit_cannot_omit_compiler_recipe(self):
        del self.a.receipt['recipe_sha256']['bundle_tools.py']
        self.a.rewrite(lambda c,e:None); self.denied()

    def test_reviewed_publication_requires_owned_probes(self):
        self.a.rewrite(lambda c,e:c.pop('tools/kf_dxprobe.exe')); self.denied()

    def test_reviewed_publication_requires_compiler_notices(self):
        self.a.rewrite(lambda c,e:c.pop('LICENSES/zig/LICENSE')); self.denied()

    def test_unlisted_guest_payload_refused(self):
        self.a.rewrite(lambda c,e:c.update({'guest/unreviewed.ps1':b'unreviewed'}));self.denied()

    def test_unlisted_package_payload_refused(self):
        self.a.rewrite(lambda c,e:c.update({'pkg/sevenzip/unreviewed.exe':b'unreviewed'}));self.denied()

    def test_guest_script_bytes_bound_to_checkout(self):
        self.a.rewrite(lambda c,e:c.update({'guest/kf_stage.ps1':b'malicious synthetic script'}));self.denied()

    def test_app_definition_bytes_bound_to_checkout(self):
        self.a.rewrite(lambda c,e:c.update({'apps.json':b'{"apps":[]}'}));self.denied()

    def test_embedded_record_cannot_add_unreviewed_payload(self):
        self.a.rewrite(lambda c,e:e.update(unreviewed='unreviewed embedded bytes'))
        self.denied()

    def test_embedded_record_cannot_change_schema(self):
        self.a.rewrite(lambda c,e:e.update(schema=2))
        self.denied()

    def test_generated_staging_bytes_bound_to_recipe(self):
        self.a.rewrite(lambda c,e:c.update({'stage.json':b'{}'}));self.denied()

    def test_generated_notice_bytes_bound_to_recipe(self):
        self.a.rewrite(lambda c,e:c.update({'THIRD_PARTY_NOTICES.txt':b'false licence notice'}));self.denied()

    def test_licence_text_is_derived_from_pinned_input(self):
        def edit(c,e):
            name=next(n for n in c if n.startswith('LICENSES/sevenzip/'))
            c[name]=b'synthetic replacement licence'
        self.a.rewrite(edit);self.denied()

    def test_generated_licence_index_is_derived_from_pinned_input(self):
        self.a.rewrite(lambda c,e:c.update({'LICENSES/index.json':b'[]'}));self.denied()

    def test_package_licence_metadata_bound_to_recipe(self):
        def edit(c,e):pass
        # The metadata is outside bundle.inputs.json and its hash must not bless itself.
        self.a.receipt['files']['pkg/sevenzip/7zr.exe']['licenses']=[dict(id='incorrect')]
        self.a.rewrite(edit);self.denied()

    def test_repository_licence_metadata_bound_to_recipe(self):
        self.a.receipt['files']['apps.json']['licenses']=[dict(id='incorrect')]
        self.a.rewrite(lambda c,e:None);self.denied()


class PrimaryNamespace(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.p=ImagePair(Path(self.tmp.name))

    def rejected(self):
        with self.assertRaises(ValueError):R.read_candidate(self.p.iso,self.p.candidate)
        rc,out=self.p.cli();self.assertEqual(rc,1);self.assertNotIn('REPRODUCTION_LEVEL1 EQUAL',out)

    def warning(self):
        candidate=R.read_candidate(self.p.iso,self.p.candidate)
        result=R.compare(self.p.original,candidate)
        self.assertEqual(result['level1'],'equal');self.assertEqual(result['level2'],'different')
        rc,out=self.p.cli();self.assertEqual(rc,0);self.assertIn('WARNING LEVEL2_CONTAINER_DIFF',out)

    def test_healthy_namespaces_are_accepted(self):
        candidate=R.read_candidate(self.p.iso,self.p.candidate)
        self.assertEqual(R.compare(self.p.original,candidate)['level1'],'equal')

    def test_primary_redirected_to_new_payload_is_not_metadata(self):
        def edit(raw):
            entry=next(iter(directory_entries(raw).values()))['entry'];body=b'primary-only evil bytes'
            extent=ImagePair.append(raw,body);raw[entry+2:entry+10]=I.both32(extent)
            raw[entry+10:entry+18]=I.both32(len(body))
        self.p.alter(edit);self.rejected()

    def test_primary_swapped_file_payloads_are_not_metadata(self):
        def edit(raw):
            entries=list(directory_entries(raw).values());a,b=[x['entry']for x in entries]
            first,second=bytes(raw[a+2:a+18]),bytes(raw[b+2:b+18]);raw[a+2:a+18]=second;raw[b+2:b+18]=first
        self.p.alter(edit);self.rejected()

    def test_unexpected_primary_file_is_not_metadata(self):
        def edit(raw):
            extent=ImagePair.append(raw,b'extra')
            entries=list(directory_entries(raw).values());last=entries[-1]['entry'];end=last+raw[last]
            row=I.record(b'F9999999.BIN;1',extent,5);raw[end:end+len(row)]=row
        self.p.alter(edit);self.rejected()

    def test_unexpected_empty_primary_file_changes_inventory(self):
        def edit(raw):
            extent = ImagePair.append(raw, b'')
            entries = list(directory_entries(raw).values())
            last = entries[-1]['entry']; end = last + raw[last]
            row = I.record(b'F9999998.BIN;1', extent, 0)
            raw[end:end+len(row)] = row
        self.p.alter(edit); self.rejected()

    def test_primary_dot_reference_cannot_point_elsewhere(self): self.bad_dot(16, False)
    def test_primary_parent_reference_cannot_point_elsewhere(self): self.bad_dot(16, True)
    def test_joliet_dot_reference_cannot_point_elsewhere(self): self.bad_dot(17, False)
    def test_joliet_parent_reference_cannot_point_elsewhere(self): self.bad_dot(17, True)

    def bad_dot(self, sector, parent):
        def edit(raw):
            start = int.from_bytes(raw[sector*I.BLOCK+158:sector*I.BLOCK+162], 'little')*I.BLOCK
            if parent: start += raw[start]
            raw[start+2:start+10] = I.both32(19)
        self.p.alter(edit); self.rejected()

    def test_changed_primary_alias_is_not_metadata(self):
        def edit(raw):
            entry=next(iter(directory_entries(raw).values()))['entry'];raw[entry+33:entry+33+raw[entry+32]]=b'F0000009.BIN;1'
        self.p.alter(edit);self.rejected()

    def test_primary_little_path_table_redirect_refused(self):self.bad_table(16,140,'little')
    def test_primary_big_path_table_redirect_refused(self):self.bad_table(16,148,'big')
    def test_joliet_little_path_table_redirect_refused(self):self.bad_table(17,140,'little')
    def test_joliet_big_path_table_redirect_refused(self):self.bad_table(17,148,'big')

    def bad_table(self,sector,offset,endian):
        def edit(raw):
            table=int.from_bytes(raw[sector*I.BLOCK+offset:sector*I.BLOCK+offset+4],endian)*I.BLOCK
            extent=int.from_bytes(raw[table+2:table+6],endian)
            raw[table+2:table+6]=(extent+1).to_bytes(4,endian)
        self.p.alter(edit);self.rejected()

    def test_primary_record_date_variation_warns(self):
        def edit(raw):
            entry=next(iter(directory_entries(raw).values()))['entry'];raw[entry+18]^=1
        self.p.alter(edit);self.warning()

    def test_primary_publisher_variation_warns(self):
        self.p.alter(lambda raw:raw.__setitem__(16*I.BLOCK+318,ord('X')));self.warning()

    def test_identical_primary_payload_at_another_extent_warns(self):
        def edit(raw):
            row=next(iter(directory_entries(raw).values()));start=row['extent']*I.BLOCK
            extent=ImagePair.append(raw,bytes(raw[start:start+row['size']]))
            raw[row['entry']+2:row['entry']+10]=I.both32(extent)
        self.p.alter(edit);self.warning()


class IsoMembers(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.iso = Path(self.tmp.name) / 'members.iso'
        I.write_image({'a': b'abcdef', 'b': b'not part of a'}, self.iso)
        self.image = I.Reader(self.iso); self.addCleanup(self.image.file.close)

    def test_read_is_bounded_by_member_size(self):
        with self.image.member('a') as member:
            self.assertEqual(member.read(10**9), b'abcdef')
            self.assertEqual(member.read(), b'')

    def test_read_after_seek_cannot_escape_member(self):
        with self.image.member('a') as member:
            member.seek(-2, 2)
            self.assertEqual(member.read(100), b'ef')
            self.assertEqual(member.tell(), 6)

    def test_negative_member_seek_refused(self):
        with self.image.member('a') as member:
            with self.assertRaises(ValueError): member.seek(-1)

    def test_past_end_member_seek_refused(self):
        with self.image.member('a') as member:
            with self.assertRaises(ValueError): member.seek(7)

    def test_write_mode_refused(self):
        with self.image.member('a') as member:
            with self.assertRaises(ValueError): member.open('wb')

    def test_append_mode_refused(self):
        with self.image.member('a') as member:
            with self.assertRaises(ValueError): member.open('ab')

    def test_zip_licences_read_directly_from_iso(self):
        raw = io.BytesIO()
        with zipfile.ZipFile(raw, 'w') as archive:
            archive.writestr('LICENSE', b'synthetic ZIP licence')
            archive.writestr('app.exe', b'unexecuted app bytes')
        self.image.file.close()
        I.write_image({'pkg/app.zip': raw.getvalue()}, self.iso)
        with I.Reader(self.iso) as image:
            result = list(B.bundle_compliance.legal_members(image.member('pkg/app.zip'), 'zip'))
        self.assertEqual(result, [('LICENSE', b'synthetic ZIP licence')])

    def test_source_licences_stream_directly_from_iso(self):
        raw = io.BytesIO(); body = b'synthetic source licence'
        with tarfile.open(fileobj=raw, mode='w:xz') as archive:
            info = tarfile.TarInfo('source/COPYING'); info.size = len(body)
            archive.addfile(info, io.BytesIO(body))
        self.image.file.close()
        I.write_image({'SOURCE/app.tar.xz': raw.getvalue()}, self.iso)
        with I.Reader(self.iso) as image:
            result = list(B.bundle_compliance.legal_members(image.member('SOURCE/app.tar.xz'), 'tar'))
        self.assertEqual(result, [('source/COPYING', body)])

    def test_closed_member_read_refused(self):
        member = self.image.member('a'); member.close()
        with self.assertRaises(ValueError): member.read()

    def test_closed_member_sized_read_refused(self):
        member = self.image.member('a'); member.close()
        with self.assertRaises(ValueError): member.read(1)
