const { Realtime, TextMessage } = AV

const App = new Vue({
  el: '#app',
  template: '#template',
  data: {
    socket: null,
    player: null,
    hls: null,
    goEasyConnect: null,
    videoList: [],
    videoSrc: '',
    playing: false,
    controlParam: {
      user: '',
      action: '',
      time: '',
    },
    userId: '',
    chatRoom: null,
    appId: 'lyfrFvyNb9gKtJAe1QGybvXF-gzGzoHsz',
    appKey: 'jf1QtS8Y52y1PhpnFE7s76RI',
    server: 'https://lyfrfvyn.lc-cn-n1-shared.com', // REST API 服务器地址
  },
  methods: {
    randomString(length) {
      let str = ''
      for (let i = 0; i < length; i++) {
        str += Math.random().toString(36).substr(2)
      }
      return str.substr(0, length)
    },
    addVideo() {
      if (this.videoSrc) {
        this.videoList.push(decodeURI(this.videoSrc))
      }
      localStorage.setItem('videoList', JSON.stringify(this.videoList))
    },
    playVideoItem(src) {
      if(src.includes('.m3u8')){
        this.hls.loadSource(src);
        this.hls.attachMedia(this.player);
      } else {
        this.$refs.video.src = src
      }
      localStorage.setItem('currentPlayVideo', src)

    },
    deleteVideoItem(index) {
      this.videoList.splice(index, 1)
      localStorage.setItem('videoList', JSON.stringify(this.videoList))
    },
    toggleFullScreen() {
      if (this.player.requestFullscreen) {
        this.player.requestFullscreen()
      } else if (this.player.mozRequestFullScreen) {
        this.player.mozRequestFullScreen()
      } else if (this.player.webkitRequestFullscreen) {
        this.player.webkitRequestFullscreen()
      } else if (this.player.msRequestFullscreen) {
        this.player.msRequestFullscreen()
      }
    },
    playVideo() {
      if (this.playing) {
        this.player.pause()
        this.controlParam.action = 'pause'
        this.controlParam.time = this.player.currentTime
        this.sendMessage(this.controlParam)
      } else {
        this.player.play()
        this.controlParam.action = 'play'
        this.controlParam.time = this.player.currentTime
        this.sendMessage(this.controlParam)
      }
    },
    seekVideo() {
      this.player.pause()
      this.controlParam.action = 'seek'
      this.controlParam.time = this.player.currentTime
      this.sendMessage(this.controlParam)
    },
    sendMessage(controlParam){
      const params = JSON.stringify(controlParam)
      this.chatRoom.send(new TextMessage(params))
    },
    resultHandler(result) {
      switch (result.action) {
        case "play":
          this.player.currentTime = (result.time + 0.2) //播放时+0.2秒，抵消网络延迟
          this.player.play();
          break
        case "pause":
          this.player.currentTime = (result.time)
          this.player.pause();
          break
        case "seek":
          this.player.currentTime = (result.time);
          break
      }
    },
    getParam(variable) {
      var query = window.location.search.substring(1);
      var vars = query.split("&");
      for (var i = 0; i < vars.length; i++) {
        var pair = vars[i].split("=");
        if (pair[0] == variable) {
          return pair[1];
        }
      }
      return false;
    },
    setParam(param,val){
      var stateObject = 0;
      var title="0"
      var oUrl = window.location.href.toString();
      var nUrl = "";
      var pattern=param+'=([^&]*)';
      var replaceText=param+'='+val; 
      if(oUrl.match(pattern)){
          var tmp='/('+ param+'=)([^&]*)/gi';
          tmp=oUrl.replace(eval(tmp),replaceText);
          nUrl = tmp;
      }else{ 
          if(oUrl.match('[\?]')){ 
            nUrl = oUrl+'&'+replaceText; 
          }else{ 
            nUrl = oUrl+'?'+replaceText; 
          } 
      }
      history.replaceState(stateObject,title,nUrl);
    }
  },
  created() {
    const localList = JSON.parse(localStorage.getItem('videoList'))

    this.videoList = localList ? localList : []

    const currentPlayVideo = localStorage.getItem('currentPlayVideo')

    if(currentPlayVideo){
      this.videoSrc = currentPlayVideo
    }

    if(this.getParam("url")){
      this.videoSrc = decodeURIComponent(this.getParam("url"))
    }

    this.userId = this.randomString(10)

    this.controlParam.user = this.userId
  },
  mounted() {

    this.player = this.$refs.video

    if (Hls.isSupported()) {
      this.hls = new Hls();
      this.hls.loadSource(this.videoSrc);
      this.hls.attachMedia(this.player);
    }
    const that = this
    const realtime = new Realtime({
      appId: this.appId,
      appKey: this.appKey,
      server: this.server,
    })
    var roomId = this.getParam("id")?this.getParam("id"):'6220af14c335ae061100cab7'
    var client, room

    realtime.createIMClient(this.userId).then(function(c) {
      console.log('连接成功')
      client = c
      client.on('disconnect', function() {
        console.log('[disconnect] 服务器连接已断开')
      })
      client.on('offline', function() {
        console.log('[offline] 离线（网络连接已断开）')
      })
      client.on('online', function() {
        console.log('[online] 已恢复在线')
      })
      client.on('schedule', function(attempt, time) {
        console.log(
          '[schedule] ' +
          time / 1000 +
          's 后进行第 ' +
          (attempt + 1) +
          ' 次重连'
        )
      })
      client.on('retry', function(attempt) {
        console.log('[retry] 正在进行第 ' + (attempt + 1) + ' 次重连')
      })
      client.on('reconnect', function() {
        console.log('[reconnect] 重连成功')
      })
      client.on('reconnecterror', function() {
        console.log('[reconnecterror] 重连失败')
      })
      return c.getConversation(roomId)
    })
      .then(function(conversation) {
        if (conversation) {
          return conversation
        } else {
          console.log('不存在这个 conversation，创建一个。')
          return client
            .createConversation({
              name: 'LeanCloud-Conversation',
              transient: true,
            })
            .then(function(conversation) {
              roomId = conversation.id
              console.log('创建新 Room 成功，id 是：', roomId)
              that.setParam("id", roomId)
              return conversation
            })
        }
      })
      .then(function(conversation) {
        return conversation.join()
      })
      .then(function(conversation) {
        room = conversation;
        that.chatRoom = conversation
        room.on('message', function(message) {
          const result = JSON.parse(message._lctext)
          that.resultHandler(result)
        });
      })
      .catch(function(err) {
        console.error(err);
        console.log('错误：' + err.message);
      });

    this.player.addEventListener('play', () => {
      this.playing = true
    })
    this.player.addEventListener('pause', () => {
      this.playing = false
    })
  }
})
