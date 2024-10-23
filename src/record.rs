#![allow(clippy::single_match)]
use std::{cell::RefCell, rc::Rc};

use pipewire::{
    context::Context,
    keys,
    main_loop::MainLoop,
    properties::properties,
    registry::GlobalObject,
    spa::{
        self,
        pod::{object, property, Pod},
        utils::dict::DictRef,
    },
    stream::{Stream, StreamFlags, StreamRef, StreamState},
};
use ringbuf::traits::Producer;

use crate::RingProd;

struct Data {
    prod: RingProd,
}

impl Data {
    fn param_changed(&mut self, _stream: &StreamRef, id: u32, param: Option<&Pod>) {
        log::info!("param changed");
        let Some(param) = param else {
            return;
        };
        match spa::param::ParamType::from_raw(id) {
            spa::param::ParamType::Format => {
                let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param)
                else {
                    return;
                };

                if media_type != spa::param::format::MediaType::Video
                    || media_subtype != spa::param::format::MediaSubtype::Raw
                {
                    return;
                }
                log::info!("got {media_type:?}/{media_subtype:?}");
            }
            _ => {}
        }
    }
    fn process(&mut self, stream: &StreamRef) {
        let Some(mut buf) = stream.dequeue_buffer() else {
            return;
        };
        let chunk = buf.datas_mut()[0].chunk();
        let size = chunk.size() as usize;
        let Some(samples) = buf.datas_mut().first_mut() else {
            return;
        };
        let Some(data) = samples.data() else {
            return;
        };
        let mut pushed = 0;
        while pushed < size {
            match self.prod.wait_vacant(size - pushed) {
                Ok(()) => {}
                Err(err) => match err {
                    ringbuf_blocking::WaitError::TimedOut => continue,
                    ringbuf_blocking::WaitError::Closed => panic!("ringbuf closed"),
                },
            }
            pushed += self.prod.push_slice(&data[pushed..size]);
        }
    }
    fn state_changed(
        &mut self,
        _stream: &StreamRef,
        _old_state: StreamState,
        _new_state: StreamState,
    ) {
        log::info!("state changed");
    }
}

struct Global {
    node_name: String,
    obj: Option<u32>,
    stream: Stream,
}

impl Global {
    fn global(&mut self, obj: &GlobalObject<&DictRef>) {
        match obj.type_ {
            pipewire::types::ObjectType::Node => {
                let Some(props) = obj.props else { return };
                log::debug!("node name: {:?}", props.get("node.name"));
                if props.get("node.name") != Some(&self.node_name) {
                    return;
                }
            }
            _ => return,
        }
        if self.obj.is_some() {
            return;
        }
        let pod_object = object! {
            spa::utils::SpaTypes::ObjectParamFormat,
            spa::param::ParamType::EnumFormat,
            property!(
                spa::param::format::FormatProperties::MediaType,
                Id,
                spa::param::format::MediaType::Audio
            ),
            property!(
                spa::param::format::FormatProperties::MediaSubtype,
                Id,
                spa::param::format::MediaSubtype::Raw
            ),
            property!(
               spa::param::format::FormatProperties::AudioFormat,
               Id,
               spa::param::audio::AudioFormat::S16LE
            ),
            property!(
               spa::param::format::FormatProperties::AudioRate,
               Int,
               48000
            ),
            property!(
               spa::param::format::FormatProperties::AudioChannels,
               Int,
               2
            ),
        };

        let values = match spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &spa::pod::Value::Object(pod_object),
        ) {
            Ok(x) => x.0.into_inner(),
            Err(err) => {
                log::error!("pod serializer: {err}");
                return;
            }
        };
        let mut params = [spa::pod::Pod::from_bytes(&values).unwrap()];

        match self.stream.connect(
            spa::utils::Direction::Input,
            Some(obj.id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut params,
        ) {
            Ok(()) => {}
            Err(err) => {
                log::error!("connect: {err}");
            }
        }
        self.obj = Some(obj.id);
    }
    fn global_remove(&mut self, obj: u32) {
        if Some(obj) != self.obj {
            return;
        }
        match self.stream.disconnect() {
            Ok(()) => {}
            Err(err) => {
                log::error!("disconnect: {err}");
            }
        }
    }
}

pub fn main(node_name: String, prod: RingProd) -> Result<(), Box<dyn std::error::Error>> {
    let mainloop = MainLoop::new(None)?;
    let context = Context::new(&mainloop)?;
    let core = context.connect(None)?;
    let registry = core.get_registry()?;

    let props = properties! {
       keys::MEDIA_TYPE.as_bytes() => "Audio",
       keys::MEDIA_CATEGORY.as_bytes() => "Capture",
       keys::MEDIA_ROLE.as_bytes() => "Music",
    };

    let stream = Stream::new(&core, "audio-capture", props)?;

    let _listener = stream
        .add_local_listener_with_user_data(Data { prod })
        .state_changed(|stream, data, old_state, new_state| {
            data.state_changed(stream, old_state, new_state)
        })
        .param_changed(|stream, data, id, param| data.param_changed(stream, id, param))
        .process(|stream, data| data.process(stream))
        .register()?;

    let global = Rc::new(RefCell::new(Global {
        node_name,
        obj: None,
        stream,
    }));
    let global2 = global.clone();

    let _listener = registry
        .add_listener_local()
        .global(move |obj| (*global).borrow_mut().global(obj))
        .global_remove(move |obj| (*global2).borrow_mut().global_remove(obj))
        .register();

    mainloop.run();

    Ok(())
}
