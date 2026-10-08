//! obs-show: the real OBS settings as the feature reads them (read-only; nothing is changed, no connection is made).
fn main() {
    use bu_obs::os::ObsOs;
    let os = bu_obs::real::RealOs::new();
    let c = bu_obs::cfg::ObsCfg::read(&os.obs_dir());
    println!("obs dir: {}", os.obs_dir().display());
    println!("OBS running: {}", os.obs_running());
    println!("websocket: known={} enabled={} json={} port={} auth={} password_set={}", c.ws_known, c.ws_enabled, c.ws_json, c.ws_port, c.ws_auth, !c.ws_password.is_empty());
    println!("profile: {}", c.profile_dir.display());
    println!("save clip keys: {:?}  replay key: {:?}  record key: {:?}", c.clip, c.rbkey, c.reckey);
    println!("clip length {} s, fps {}/{}, advanced {}, recording folder {}, kbps {}", c.rb_sec, c.fps_num, c.fps_den, c.adv, c.rec_dir.display(), c.kbps);
    println!("Notifications for OBS running on its own: {:?}", os.other_app());
    println!("its Start with Windows exe: {:?}", os.other_app_run_entry());
}
