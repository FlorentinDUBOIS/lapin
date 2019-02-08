use amq_protocol::protocol::{AMQPClass, access, basic, channel, confirm, exchange, queue};
use log::{error, trace};

use std::collections::HashSet;

use crate::connection::*;
use crate::consumer::*;
use crate::queue::*;
use crate::message::*;
use crate::error::*;
use crate::types::*;

#[derive(Clone,Debug,PartialEq,Eq)]
pub enum ChannelState {
    Initial,
    Connected,
    Closed,
    Error,
    SendingContent(usize),
    WillReceiveContent(String,Option<String>),
    ReceivingContent(String,Option<String>,usize),
}

pub type RequestId = u64;

#[derive(Debug)]
pub enum Answer {
    AwaitingChannelOpenOk(RequestId),
    AwaitingChannelFlowOk(RequestId),
    AwaitingChannelCloseOk(RequestId),

    AwaitingAccessRequestOk(RequestId),

    AwaitingExchangeDeclareOk(RequestId),
    AwaitingExchangeDeleteOk(RequestId),
    AwaitingExchangeBindOk(RequestId),
    AwaitingExchangeUnbindOk(RequestId),

    AwaitingQueueDeclareOk(RequestId),
    AwaitingQueueBindOk(RequestId, String, String),
    AwaitingQueuePurgeOk(RequestId, String),
    AwaitingQueueDeleteOk(RequestId, String),
    AwaitingQueueUnbindOk(RequestId, String, String),

    AwaitingBasicQosOk(RequestId, u32,u16,bool),
    AwaitingBasicConsumeOk(RequestId, String, String, bool, bool, bool, bool, Box<dyn ConsumerSubscriber>),
    AwaitingBasicCancelOk(RequestId),
    AwaitingBasicGetAnswer(RequestId, String),
    AwaitingBasicRecoverOk(RequestId),

    AwaitingTxSelectOk(RequestId),
    AwaitingTxCommitOk(RequestId),
    AwaitingTxRollbackOk(RequestId),

    // RabbitMQ confirm extension
    AwaitingConfirmSelectOk(RequestId),
    AwaitingPublishConfirm(RequestId),
}

impl Connection {
    pub fn receive_method(&mut self, channel_id: u16, method: AMQPClass) -> Result<(), Error> {
        match method {

            AMQPClass::Channel(channel::AMQPMethod::OpenOk(m)) => {
                self.receive_channel_open_ok(channel_id, m)
            }
            AMQPClass::Channel(channel::AMQPMethod::Flow(m)) => self.receive_channel_flow(channel_id, m),
            AMQPClass::Channel(channel::AMQPMethod::FlowOk(m)) => {
                self.receive_channel_flow_ok(channel_id, m)
            }
            AMQPClass::Channel(channel::AMQPMethod::Close(m)) => self.receive_channel_close(channel_id, m),
            AMQPClass::Channel(channel::AMQPMethod::CloseOk(m)) => {
                self.receive_channel_close_ok(channel_id, m)
            }

            AMQPClass::Access(access::AMQPMethod::RequestOk(m)) => {
                self.receive_access_request_ok(channel_id, m)
            }

            AMQPClass::Exchange(exchange::AMQPMethod::DeclareOk(m)) => {
                self.receive_exchange_declare_ok(channel_id, m)
            }
            AMQPClass::Exchange(exchange::AMQPMethod::DeleteOk(m)) => {
                self.receive_exchange_delete_ok(channel_id, m)
            }
            AMQPClass::Exchange(exchange::AMQPMethod::BindOk(m)) => {
                self.receive_exchange_bind_ok(channel_id, m)
            }
            AMQPClass::Exchange(exchange::AMQPMethod::UnbindOk(m)) => {
                self.receive_exchange_unbind_ok(channel_id, m)
            }

            AMQPClass::Queue(queue::AMQPMethod::DeclareOk(m)) => {
                self.receive_queue_declare_ok(channel_id, m)
            }
            AMQPClass::Queue(queue::AMQPMethod::BindOk(m)) => self.receive_queue_bind_ok(channel_id, m),
            AMQPClass::Queue(queue::AMQPMethod::PurgeOk(m)) => self.receive_queue_purge_ok(channel_id, m),
            AMQPClass::Queue(queue::AMQPMethod::DeleteOk(m)) => {
                self.receive_queue_delete_ok(channel_id, m)
            }
            AMQPClass::Queue(queue::AMQPMethod::UnbindOk(m)) => {
                self.receive_queue_unbind_ok(channel_id, m)
            }

            AMQPClass::Basic(basic::AMQPMethod::QosOk(m)) => self.receive_basic_qos_ok(channel_id, m),
            AMQPClass::Basic(basic::AMQPMethod::ConsumeOk(m)) => {
                self.receive_basic_consume_ok(channel_id, m)
            }
            AMQPClass::Basic(basic::AMQPMethod::CancelOk(m)) => {
                self.receive_basic_cancel_ok(channel_id, m)
            }
            AMQPClass::Basic(basic::AMQPMethod::Deliver(m)) => self.receive_basic_deliver(channel_id, m),
            AMQPClass::Basic(basic::AMQPMethod::GetOk(m)) => self.receive_basic_get_ok(channel_id, m),
            AMQPClass::Basic(basic::AMQPMethod::GetEmpty(m)) => {
                self.receive_basic_get_empty(channel_id, m)
            }
            AMQPClass::Basic(basic::AMQPMethod::RecoverOk(m)) => {
                self.receive_basic_recover_ok(channel_id, m)
            }

            /*
            AMQPClass::Tx(tx::Methods::SelectOk(m)) => self.receive_tx_select_ok(channel_id, m),
            AMQPClass::Tx(tx::Methods::CommitOk(m)) => self.receive_tx_commit_ok(channel_id, m),
            AMQPClass::Tx(tx::Methods::RollbackOk(m)) => self.receive_tx_rollback_ok(channel_id, m),
            */

            AMQPClass::Confirm(confirm::AMQPMethod::SelectOk(m)) => {
                self.receive_confirm_select_ok(channel_id, m)
            }
            AMQPClass::Basic(basic::AMQPMethod::Ack(m)) => {
                self.receive_basic_ack(channel_id, m)
            }
            AMQPClass::Basic(basic::AMQPMethod::Nack(m)) => {
                self.receive_basic_nack(channel_id, m)
            }

            m => {
                error!("the client should not receive this method: {:?}", m);
                return Err(ErrorKind::InvalidMethod(m).into());
            }
        }
    }

    pub fn channel_open(&mut self,
                        _channel_id: u16,
                        out_of_band: ShortString)
                        -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if let Err(err) = self.check_state(_channel_id, ChannelState::Initial) {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(err);
        }

        let method =
            AMQPClass::Channel(channel::AMQPMethod::Open(channel::Open { out_of_band: out_of_band }));

        self.send_method_frame(_channel_id, method).map(|_| {
            trace!("channel[{}] setting state to ChannelState::AwaitingChannelOpenOk", _channel_id);
            let request_id = self.next_request_id();
            self.push_back_answer(_channel_id, Answer::AwaitingChannelOpenOk(request_id));
            request_id
        })
    }

    pub fn receive_channel_open_ok(&mut self,
                                   _channel_id: u16,
                                   _: channel::OpenOk)
                                   -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if let Err(err) = self.check_state(_channel_id, ChannelState::Initial) {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(err);
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingChannelOpenOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }

        self.set_channel_state(_channel_id, ChannelState::Connected);
        Ok(())
    }

    pub fn channel_flow(&mut self, _channel_id: u16, active: Boolean) -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Channel(channel::AMQPMethod::Flow(channel::Flow { active: active }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.push_back_answer(_channel_id, Answer::AwaitingChannelFlowOk(request_id));
            request_id
        })
    }

    pub fn receive_channel_flow(&mut self,
                                _channel_id: u16,
                                method: channel::Flow)
                                -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        self.channels.get_mut(&_channel_id).map(|c| c.send_flow = method.active);
        self.channel_flow_ok(_channel_id, method.active)
    }

    pub fn channel_flow_ok(&mut self, _channel_id: u16, active: Boolean) -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Channel(channel::AMQPMethod::FlowOk(channel::FlowOk { active: active }));
        self.send_method_frame(_channel_id, method)
    }

    pub fn receive_channel_flow_ok(&mut self,
                                   _channel_id: u16,
                                   method: channel::FlowOk)
                                   -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingChannelFlowOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            self.channels.get_mut(&_channel_id).map(|c| c.receive_flow = method.active);
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }

        Ok(())
    }

    pub fn channel_close(&mut self,
                         _channel_id: u16,
                         reply_code: ShortUInt,
                         reply_text: ShortString,
                         class_id: ShortUInt,
                         method_id: ShortUInt)
                         -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Channel(channel::AMQPMethod::Close(channel::Close {
            reply_code: reply_code,
            reply_text: reply_text,
            class_id: class_id,
            method_id: method_id,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
          let request_id = self.next_request_id();
          self.push_back_answer(_channel_id, Answer::AwaitingChannelCloseOk(request_id));
          request_id
        })
    }

    pub fn receive_channel_close(&mut self,
                                 _channel_id: u16,
                                 _: channel::Close)
                                 -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        //FIXME: log the error if there is one
        //FIXME: handle reply codes

        self.get_next_answer(_channel_id);
        self.set_channel_state(_channel_id, ChannelState::Closed);
        self.channel_close_ok(_channel_id)
    }

    pub fn channel_close_ok(&mut self, _channel_id: u16) -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if ! self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Channel(channel::AMQPMethod::CloseOk(channel::CloseOk {}));
        self.send_method_frame(_channel_id, method)
    }

    pub fn receive_channel_close_ok(&mut self,
                                    _channel_id: u16,
                                    _: channel::CloseOk)
                                    -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if ! self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingChannelCloseOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            self.set_channel_state(_channel_id, ChannelState::Closed);
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }

        Ok(())
    }

    pub fn access_request(&mut self,
                          _channel_id: u16,
                          realm: ShortString,
                          exclusive: Boolean,
                          passive: Boolean,
                          active: Boolean,
                          write: Boolean,
                          read: Boolean)
                          -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if ! self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Access(access::AMQPMethod::Request(access::Request {
            realm: realm,
            exclusive: exclusive,
            passive: passive,
            active: active,
            write: write,
            read: read,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingAccessRequestOk(request_id));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }


    pub fn receive_access_request_ok(&mut self,
                                     _channel_id: u16,
                                     _: access::RequestOk)
                                     -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingAccessRequestOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn exchange_declare(&mut self,
                            _channel_id: u16,
                            ticket: ShortUInt,
                            exchange: ShortString,
                            exchange_type: ShortString,
                            passive: Boolean,
                            durable: Boolean,
                            auto_delete: Boolean,
                            internal: Boolean,
                            nowait: Boolean,
                            arguments: FieldTable)
                            -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Exchange(exchange::AMQPMethod::Declare(exchange::Declare {
            ticket: ticket,
            exchange: exchange,
            type_: exchange_type,
            passive: passive,
            durable: durable,
            auto_delete: auto_delete,
            internal: internal,
            nowait: nowait,
            arguments: arguments,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
          let request_id = self.next_request_id();
          self.channels.get_mut(&_channel_id).map(|c| {
              c.awaiting.push_back(Answer::AwaitingExchangeDeclareOk(request_id));
              trace!("channel {} state is now {:?}", _channel_id, c.state);
          });
          request_id
        })
    }

    pub fn receive_exchange_declare_ok(&mut self,
                                       _channel_id: u16,
                                       _: exchange::DeclareOk)
                                       -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingExchangeDeclareOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }


    pub fn exchange_delete(&mut self,
                           _channel_id: u16,
                           ticket: ShortUInt,
                           exchange: ShortString,
                           if_unused: Boolean,
                           nowait: Boolean)
                           -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Exchange(exchange::AMQPMethod::Delete(exchange::Delete {
            ticket: ticket,
            exchange: exchange,
            if_unused: if_unused,
            nowait: nowait,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
          let request_id = self.next_request_id();
          self.channels.get_mut(&_channel_id).map(|c| {
              c.awaiting.push_back(Answer::AwaitingExchangeDeleteOk(request_id));
              trace!("channel {} state is now {:?}", _channel_id, c.state);
          });
          request_id
        })
    }

    pub fn receive_exchange_delete_ok(&mut self,
                                      _channel_id: u16,
                                      _: exchange::DeleteOk)
                                      -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingExchangeDeleteOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn exchange_bind(&mut self,
                         _channel_id: u16,
                         ticket: ShortUInt,
                         destination: ShortString,
                         source: ShortString,
                         routing_key: ShortString,
                         nowait: Boolean,
                         arguments: FieldTable)
                         -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Exchange(exchange::AMQPMethod::Bind(exchange::Bind {
            ticket: ticket,
            destination: destination,
            source: source,
            routing_key: routing_key,
            nowait: nowait,
            arguments: arguments,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingExchangeBindOk(request_id));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_exchange_bind_ok(&mut self,
                                    _channel_id: u16,
                                    _: exchange::BindOk)
                                    -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingExchangeBindOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn exchange_unbind(&mut self,
                           _channel_id: u16,
                           ticket: ShortUInt,
                           destination: ShortString,
                           source: ShortString,
                           routing_key: ShortString,
                           nowait: Boolean,
                           arguments: FieldTable)
                           -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Exchange(exchange::AMQPMethod::Unbind(exchange::Unbind {
            ticket: ticket,
            destination: destination,
            source: source,
            routing_key: routing_key,
            nowait: nowait,
            arguments: arguments,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingExchangeUnbindOk(request_id));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_exchange_unbind_ok(&mut self,
                                      _channel_id: u16,
                                      _: exchange::UnbindOk)
                                      -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingExchangeUnbindOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn queue_declare(&mut self,
                         _channel_id: u16,
                         ticket: ShortUInt,
                         queue: ShortString,
                         passive: Boolean,
                         durable: Boolean,
                         exclusive: Boolean,
                         auto_delete: Boolean,
                         nowait: Boolean,
                         arguments: FieldTable)
                         -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Queue(queue::AMQPMethod::Declare(queue::Declare {
            ticket: ticket,
            queue: queue.clone(),
            passive: passive,
            durable: durable,
            exclusive: exclusive,
            auto_delete: auto_delete,
            nowait: nowait,
            arguments: arguments,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
          let request_id = self.next_request_id();
          self.channels.get_mut(&_channel_id).map(|c| {
            c.awaiting.push_back(Answer::AwaitingQueueDeclareOk(request_id));
            trace!("channel {} state is now {:?}", _channel_id, c.state);
          });
          request_id
        })
    }

    pub fn receive_queue_declare_ok(&mut self,
                                    _channel_id: u16,
                                    method: queue::DeclareOk)
                                    -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingQueueDeclareOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            self.generated_names.insert(request_id, method.queue.clone());
            self.channels.get_mut(&_channel_id).map(|c| {
              let q = Queue::new(method.queue.clone(), method.message_count, method.consumer_count);
              c.queues.insert(method.queue.clone(), q);
            });
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn queue_bind(&mut self,
                      _channel_id: u16,
                      ticket: ShortUInt,
                      queue: ShortString,
                      exchange: ShortString,
                      routing_key: ShortString,
                      nowait: Boolean,
                      arguments: FieldTable)
                      -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Queue(queue::AMQPMethod::Bind(queue::Bind {
            ticket: ticket,
            queue: queue.clone(),
            exchange: exchange.clone(),
            routing_key: routing_key.clone(),
            nowait: nowait,
            arguments: arguments,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                let key = (exchange.clone(), routing_key.clone());
                c.awaiting.push_back(Answer::AwaitingQueueBindOk(request_id, exchange.clone(), routing_key.clone()));
                c.queues.get_mut(&queue).map(|q| {
                  q.bindings.insert(key, Binding::new(exchange, routing_key, nowait)
                  );
                });
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_queue_bind_ok(&mut self,
                                 _channel_id: u16,
                                 _: queue::BindOk)
                                 -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingQueueBindOk(request_id, exchange, routing_key)) => {
            self.finished_reqs.insert(request_id, true);
            let key = (exchange, routing_key);
            self.channels.get_mut(&_channel_id).map(|c| {
              for ref mut q in c.queues.values_mut() {
                q.bindings.get_mut(&key).map(|b| b.active = true);
              }
            });
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn queue_purge(&mut self,
                       _channel_id: u16,
                       ticket: ShortUInt,
                       queue: ShortString,
                       nowait: Boolean)
                       -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Queue(queue::AMQPMethod::Purge(queue::Purge {
            ticket: ticket,
            queue: queue.clone(),
            nowait: nowait,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingQueuePurgeOk(request_id, queue.clone()));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_queue_purge_ok(&mut self,
                                  _channel_id: u16,
                                  _: queue::PurgeOk)
                                  -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }


        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingQueuePurgeOk(request_id, _)) => {
            self.finished_reqs.insert(request_id, true);
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn queue_delete(&mut self,
                        _channel_id: u16,
                        ticket: ShortUInt,
                        queue: ShortString,
                        if_unused: Boolean,
                        if_empty: Boolean,
                        nowait: Boolean)
                        -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Queue(queue::AMQPMethod::Delete(queue::Delete {
            ticket: ticket,
            queue: queue.clone(),
            if_unused: if_unused,
            if_empty: if_empty,
            nowait: nowait,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingQueueDeleteOk(request_id, queue));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_queue_delete_ok(&mut self,
                                   _channel_id: u16,
                                   _: queue::DeleteOk)
                                   -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingQueueDeleteOk(request_id, key)) => {
            self.finished_reqs.insert(request_id, true);
            self.channels.get_mut(&_channel_id).map(|c| c.queues.remove(&key));
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn queue_unbind(&mut self,
                        _channel_id: u16,
                        ticket: ShortUInt,
                        queue: ShortString,
                        exchange: ShortString,
                        routing_key: ShortString,
                        arguments: FieldTable)
                        -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Queue(queue::AMQPMethod::Unbind(queue::Unbind {
            ticket: ticket,
            queue: queue,
            exchange: exchange.clone(),
            routing_key: routing_key.clone(),
            arguments: arguments,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
              c.awaiting.push_back(Answer::AwaitingQueueUnbindOk(request_id, exchange, routing_key));
              trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_queue_unbind_ok(&mut self,
                                   _channel_id: u16,
                                   _: queue::UnbindOk)
                                   -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingQueueUnbindOk(request_id, exchange, routing_key)) => {
            self.finished_reqs.insert(request_id, true);
            let key = (exchange, routing_key);
            self.channels.get_mut(&_channel_id).map(|c| {
              for ref mut q in c.queues.values_mut() {
                q.bindings.remove(&key);
              }
            });
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn basic_qos(&mut self,
                     _channel_id: u16,
                     prefetch_size: LongUInt,
                     prefetch_count: ShortUInt,
                     global: Boolean)
                     -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Qos(basic::Qos {
            prefetch_size: prefetch_size,
            prefetch_count: prefetch_count,
            global: global,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingBasicQosOk(request_id, prefetch_size, prefetch_count, global));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_basic_qos_ok(&mut self,
                                _channel_id: u16,
                                _: basic::QosOk)
                                -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingBasicQosOk(request_id, prefetch_size, prefetch_count, global)) => {
            self.finished_reqs.insert(request_id, true);
            if global {
              self.prefetch_size  = prefetch_size;
              self.prefetch_count = prefetch_count;
            } else {
              self.channels.get_mut(&_channel_id).map(|c| {
                c.prefetch_size  = prefetch_size;
                c.prefetch_count = prefetch_count;
              });
            }
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn basic_consume(&mut self,
                         _channel_id: u16,
                         ticket: ShortUInt,
                         queue: ShortString,
                         consumer_tag: ShortString,
                         no_local: Boolean,
                         no_ack: Boolean,
                         exclusive: Boolean,
                         nowait: Boolean,
                         arguments: FieldTable,
                         subscriber: Box<dyn ConsumerSubscriber>)
                         -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Consume(basic::Consume {
            ticket: ticket,
            queue: queue.clone(),
            consumer_tag: consumer_tag.clone(),
            no_local: no_local,
            no_ack: no_ack,
            exclusive: exclusive,
            nowait: nowait,
            arguments: arguments,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingBasicConsumeOk(
                  request_id, queue, consumer_tag, no_local, no_ack, exclusive, nowait, subscriber
                ));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_basic_consume_ok(&mut self,
                                    _channel_id: u16,
                                    method: basic::ConsumeOk)
                                    -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingBasicConsumeOk(request_id, queue, _, no_local, no_ack, exclusive, nowait, subscriber)) => {
            self.finished_reqs.insert(request_id, true);
            self.generated_names.insert(request_id, method.consumer_tag.clone());
            self.channels.get_mut(&_channel_id).map(|c| {
              c.queues.get_mut(&queue).map(|q| {
                let consumer = Consumer::new(method.consumer_tag.clone(), no_local, no_ack, exclusive, nowait, subscriber);
                q.consumers.insert(
                  method.consumer_tag.clone(),
                  consumer
                  )
              })
            });
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn basic_cancel(&mut self,
                        _channel_id: u16,
                        consumer_tag: ShortString,
                        nowait: Boolean)
                        -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Cancel(basic::Cancel {
            consumer_tag: consumer_tag,
            nowait: nowait,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingBasicCancelOk(request_id));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_basic_cancel_ok(&mut self,
                                   _channel_id: u16,
                                   method: basic::CancelOk)
                                   -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingBasicCancelOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            if let Some(channel) = self.channels.get_mut(&_channel_id) {
              for queue in channel.queues.values_mut() {
                queue.consumers.remove(&method.consumer_tag).map(|mut consumer| consumer.cancel());
              }
            }
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn basic_publish(&mut self,
                         _channel_id: u16,
                         ticket: ShortUInt,
                         exchange: ShortString,
                         routing_key: ShortString,
                         mandatory: Boolean,
                         immediate: Boolean)
                         -> Result<u64, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Publish(basic::Publish {
            ticket: ticket,
            exchange: exchange,
            routing_key: routing_key,
            mandatory: mandatory,
            immediate: immediate,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            //FIXME: if we're not on a confirm channel, we're jumping over some request id
            // this is not a big issue, since we only need them to be unique
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
              if c.confirm {
                c.awaiting.push_back(Answer::AwaitingPublishConfirm(request_id));
                let delivery_tag = c.message_count;
                c.unacked.insert(delivery_tag);
                c.message_count += 1;
                delivery_tag
              } else { 0 }
            }).unwrap_or(0)
        })
    }

    pub fn receive_basic_deliver(&mut self,
                                 _channel_id: u16,
                                 method: basic::Deliver)
                                 -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        self.channels.get_mut(&_channel_id).map(|c| {
            for (ref queue_name, ref mut q) in &mut c.queues {
              c.state = ChannelState::WillReceiveContent(queue_name.to_string(), Some(method.consumer_tag.to_string()));
              q.consumers.get_mut(&method.consumer_tag).map(|cs| {
                cs.current_message = Some(Delivery::new(
                  method.delivery_tag,
                  method.exchange.to_string(),
                  method.routing_key.to_string(),
                  method.redelivered
                ));
              });
            }
            trace!("channel {} state is now {:?}", _channel_id, c.state);
        });
        Ok(())
    }

    pub fn basic_get(&mut self,
                     _channel_id: u16,
                     ticket: ShortUInt,
                     queue: ShortString,
                     no_ack: Boolean)
                     -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Get(basic::Get {
            ticket: ticket,
            queue: queue.clone(),
            no_ack: no_ack,
        }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingBasicGetAnswer(request_id, queue.clone()));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_basic_get_ok(&mut self,
                                _channel_id: u16,
                                method: basic::GetOk)
                                -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingBasicGetAnswer(request_id, queue_name)) => {
            self.finished_get_reqs.insert(request_id, true);
            self.set_channel_state(_channel_id, ChannelState::WillReceiveContent(queue_name.to_string(), None));

            self.channels.get_mut(&_channel_id).map(|c| {
              c.queues.get_mut(&queue_name).map(|q| {
                q.current_get_message = Some(BasicGetMessage::new(
                  method.delivery_tag,
                  method.exchange.to_string(),
                  method.routing_key.to_string(),
                  method.redelivered,
                  method.message_count
                ));
              })
            });

            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn receive_basic_get_empty(&mut self,
                                   _channel_id: u16,
                                   _: basic::GetEmpty)
                                   -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingBasicGetAnswer(request_id, _)) => {
            self.finished_get_reqs.insert(request_id, false);
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn basic_ack(&mut self,
                     _channel_id: u16,
                     delivery_tag: LongLongUInt,
                     multiple: Boolean)
                     -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Ack(basic::Ack {
            delivery_tag: delivery_tag,
            multiple: multiple,
        }));
        let res = self.send_method_frame(_channel_id, method);
        if multiple && delivery_tag == 0 {
          self.drop_prefetched_messages(_channel_id);
        }
        res
    }

    pub fn basic_reject(&mut self,
                        _channel_id: u16,
                        delivery_tag: LongLongUInt,
                        requeue: Boolean)
                        -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Reject(basic::Reject {
            delivery_tag: delivery_tag,
            requeue: requeue,
        }));
        self.send_method_frame(_channel_id, method)
    }

    fn drop_prefetched_messages(&mut self, channel_id: u16) {
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            for queue in channel.queues.values_mut() {
                for consumer in queue.consumers.values_mut() {
                    consumer.drop_prefetched_messages();
                }
            }
        }
    }

    pub fn basic_recover_async(&mut self, _channel_id: u16, requeue: Boolean) -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method =
            AMQPClass::Basic(basic::AMQPMethod::RecoverAsync(basic::RecoverAsync { requeue: requeue }));
        let res = self.send_method_frame(_channel_id, method);
        self.drop_prefetched_messages(_channel_id);
        res
    }

    pub fn basic_recover(&mut self, _channel_id: u16, requeue: Boolean) -> Result<RequestId, Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Recover(basic::Recover { requeue: requeue }));

        let res = self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingBasicRecoverOk(request_id));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        });
        self.drop_prefetched_messages(_channel_id);
        res
    }

    pub fn receive_basic_recover_ok(&mut self,
                                    _channel_id: u16,
                                    _: basic::RecoverOk)
                                    -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingBasicRecoverOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            error!("unimplemented method Basic.RecoverOk, ignoring packet");
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn basic_nack(&mut self,
                      _channel_id: u16,
                      delivery_tag: LongLongUInt,
                      multiple: Boolean,
                      requeue: Boolean)
                      -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Basic(basic::AMQPMethod::Nack(basic::Nack {
            delivery_tag: delivery_tag,
            multiple: multiple,
            requeue: requeue,
        }));
        let res = self.send_method_frame(_channel_id, method);
        if multiple && delivery_tag == 0 {
          self.drop_prefetched_messages(_channel_id);
        }
        res
    }

    pub fn confirm_select(&mut self, _channel_id: u16, nowait: Boolean) -> Result<RequestId, Error> {
        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        let method = AMQPClass::Confirm(confirm::AMQPMethod::Select(confirm::Select { nowait: nowait }));

        self.send_method_frame(_channel_id, method).map(|_| {
            let request_id = self.next_request_id();
            self.channels.get_mut(&_channel_id).map(|c| {
                c.awaiting.push_back(Answer::AwaitingConfirmSelectOk(request_id));
                trace!("channel {} state is now {:?}", _channel_id, c.state);
            });
            request_id
        })
    }

    pub fn receive_confirm_select_ok(&mut self,
                                     _channel_id: u16,
                                     _: confirm::SelectOk)
                                     -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            trace!("key {} not in channels {:?}", _channel_id, self.channels);
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingConfirmSelectOk(request_id)) => {
            self.finished_reqs.insert(request_id, true);
            self.channels.get_mut(&_channel_id).map(|c| {
              c.confirm = true;
              c.message_count = 1;
            });
            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn receive_basic_ack(&mut self,
                     _channel_id: u16,
                     method: basic::Ack)
                     -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingPublishConfirm(request_id)) => {
            self.finished_reqs.insert(request_id, true);

            self.channels.get_mut(&_channel_id).map(|c| {
              if c.confirm {
                if method.multiple {
                  let h: HashSet<u64> = c.unacked.iter().filter(|elem| *elem <= &method.delivery_tag).cloned().collect();
                  c.unacked = c.unacked.difference(&h).cloned().collect();
                  c.acked = c.acked.union(&h).cloned().collect();
                } else {
                  if c.unacked.remove(&method.delivery_tag) {
                    c.acked.insert(method.delivery_tag);
                  }
                }
              }
            });

            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

    pub fn receive_basic_nack(&mut self,
                      _channel_id: u16,
                      method: basic::Nack)
                      -> Result<(), Error> {

        if !self.channels.contains_key(&_channel_id) {
            return Err(ErrorKind::InvalidChannel(_channel_id).into());
        }

        if !self.is_connected(_channel_id) {
            return Err(ErrorKind::NotConnected.into());
        }

        match self.get_next_answer(_channel_id) {
          Some(Answer::AwaitingPublishConfirm(request_id)) => {
            self.finished_reqs.insert(request_id, true);

            self.channels.get_mut(&_channel_id).map(|c| {
              if c.confirm {
                if method.multiple {
                  let h: HashSet<u64> = c.unacked.iter().filter(|elem| *elem <= &method.delivery_tag).cloned().collect();
                  c.unacked = c.unacked.difference(&h).cloned().collect();
                  c.acked = c.nacked.union(&h).cloned().collect();
                } else {
                  if c.unacked.remove(&method.delivery_tag) {
                    c.nacked.insert(method.delivery_tag);
                  }
                }
              }
            });

            Ok(())
          },
          _ => {
            self.set_channel_state(_channel_id, ChannelState::Error);
            return Err(ErrorKind::UnexpectedAnswer.into());
          }
        }
    }

}
