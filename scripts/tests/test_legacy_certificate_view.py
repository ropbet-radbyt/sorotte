from __future__ import annotations
import importlib.util
import pathlib
import unittest

from cryptography import x509
from cryptography.hazmat.primitives import serialization

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("legacy_peer_probe", ROOT / "crates/sorotte-compat/scripts/python_live_peer_probe.py")
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


class LegacyCertificateViewTests(unittest.TestCase):
    def test_pinned_fixture_preserves_subject_issuer_expiry_presentation(self):
        certificate = x509.load_pem_x509_certificate((ROOT / "fixtures/tls/test_cert.pem").read_bytes())
        view = probe._LegacyPeerCertificate(certificate.public_bytes(serialization.Encoding.DER))
        self.assertEqual(view.get_extension_count(), 1)
        self.assertEqual(view.get_extension(0).get_short_name(), b"subjectAltName")
        self.assertEqual(str(view.get_extension(0)), "DNS:localhost")
        self.assertEqual(view.get_issuer().CN, "localhost")
        self.assertEqual(view.get_notAfter(), b"20360215060156Z")

    def test_invalid_certificate_is_rejected(self):
        with self.assertRaises(ValueError):
            probe._LegacyPeerCertificate(b"not a certificate")

    def test_san_presentation_preserves_multiple_names(self):
        import ipaddress
        names = x509.SubjectAlternativeName([x509.DNSName("localhost"), x509.IPAddress(ipaddress.ip_address("127.0.0.1"))])
        self.assertEqual(str(probe._LegacySubjectAlternativeName(names)), "DNS:localhost, IP Address:127.0.0.1")

    def test_socket_still_requires_ca_and_hostname_verification(self):
        # This is a guard on the fixture's trust boundary; the required-live
        # TLS cases exercise actual trusted, untrusted and rotation handshakes.
        import inspect
        source = inspect.getsource(probe)
        self.assertIn("ssl.create_default_context(cafile=options.ca_file)", source)
        self.assertIn("context.check_hostname = True", source)
        self.assertIn("server_hostname=options.hostname", source)
        self.assertNotIn("CERT_NONE", source)


if __name__ == "__main__":
    unittest.main()
