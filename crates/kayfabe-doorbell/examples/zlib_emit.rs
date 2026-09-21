fn main() {
    let mut m = kayfabe_doorbell::accessmap::AccessMap::deny_all();
    m.allow_range(0x810000, 0x10000);
    let g = m.to_gzip_deflate();
    std::io::Write::write_all(&mut std::fs::File::create("/tmp/map.gz").unwrap(), &g).unwrap();
    std::io::Write::write_all(&mut std::fs::File::create("/tmp/map.raw").unwrap(), m.raw()).unwrap();
    println!("gzip bytes={} raw bytes={}", g.len(), m.raw().len());
}
