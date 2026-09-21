fn main() {
    let mut m = kayfabe_doorbell::accessmap::AccessMap::deny_all();
    m.allow_range(0x810000, 0x10000);
    let z = m.to_zlib_stored();
    std::io::Write::write_all(&mut std::fs::File::create("/tmp/map.z").unwrap(), &z).unwrap();
    std::io::Write::write_all(&mut std::fs::File::create("/tmp/map.raw").unwrap(), m.raw()).unwrap();
    println!("zlib bytes={} raw bytes={}", z.len(), m.raw().len());
}
