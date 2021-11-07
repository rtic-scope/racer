cd /home/yatekii/repos/probe-rs/racer && cargo build
cp /home/yatekii/repos/probe-rs/racer/target/debug/racer /home/yatekii/repos/probe-rs/racer/target/debug/rtic-scope-frontend-web 
export PATH=$PATH:/home/yatekii/repos/probe-rs/racer/target/debug
cd /home/yatekii/repos/probe-rs/cargo-rtic-scope/examples && cargo rtic-scope --frontend=web replay --bin blinky --trace-dir=. 0