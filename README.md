## Weesels

Weesels is a Weechat [relay client](https://weechat.org/files/doc/devel/weechat_relay_protocol.en.html) written in rust.

You probably got here by mistake, but in case not, I suggest you do not rely
too heavily on it; it is prototype quality at best. It will likely crash,
catch fire, implode and give you the flu. You have been warned.

It exists because:

1. I wanted to experiment with rust async.
1. I wanted to experiment with serde.
1. I was unhappy with current weechat remote clients and BNC alternatives.
1. TUI are beautiful.
1. There should be desktop notifications.
1. Desktop notification should cancel by themselves when irrelevant.
1. Chat applications should not use electron, nor should take 800Mb ram.


## Running

    $ cp weesels.conf.example ~/.config/weesels.conf
    $ edit ~/.config/weesels.conf
    $ cargo run


## TODO

- [x] connect
- [x] list buffers
- [x] list buffer content
- [x] input
- [x] show hotlist
- [x] desktop notifications
- [x] scroll back history
- [x] tab completion
- [ ] nick list
- [ ] [colors](https://weechat.org/files/doc/devel/weechat_dev.en.html#color_codes_in_strings)
- [ ] completion cycle/menu or suggest
- [ ] initial configuration wizard
- [ ] configurable input bindings
- [ ] more tests
- [ ] unencrypted connection (for no certs)
- [ ] insecure connection (for self-signed certs)
- [ ] reconnections with backoff interval
- [ ] configurable logging
- [ ] ui module could use some structure
